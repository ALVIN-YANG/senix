use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use arc_swap::ArcSwap;
use parking_lot::Mutex;

use crate::{
    GatewayConfig, HealthCheckConfig, HealthState, InstanceState, PersistedInstanceState, Result,
    TrafficState, UpstreamTlsConfig, error::Error,
};

#[derive(Debug)]
struct InstanceGate {
    generation: u64,
    weight: u32,
    traffic: TrafficState,
    health: HealthState,
    health_override: bool,
    in_flight: u64,
    long_lived_in_flight: u64,
}

#[derive(Debug)]
struct LiveInstance {
    address: SocketAddr,
    tls: Option<UpstreamTlsConfig>,
    health_check: Option<HealthCheckConfig>,
    control: Arc<InstanceControl>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthTarget {
    pub id: String,
    pub address: SocketAddr,
    pub tls: Option<UpstreamTlsConfig>,
    pub check: HealthCheckConfig,
}

#[derive(Debug)]
struct InstanceControl {
    id: String,
    gate: Mutex<InstanceGate>,
}

impl LiveInstance {
    fn state(&self) -> InstanceState {
        let gate = self.control.gate.lock();
        InstanceState {
            id: self.control.id.clone(),
            generation: gate.generation,
            weight: gate.weight,
            traffic: gate.traffic,
            health: gate.health,
            health_override: gate.health_override,
            in_flight: gate.in_flight,
            long_lived_in_flight: gate.long_lived_in_flight,
        }
    }

    fn selectable_weight(&self) -> u32 {
        let gate = self.control.gate.lock();
        if gate.traffic == TrafficState::Serving
            && (gate.health == HealthState::Healthy || gate.health_override)
        {
            gate.weight
        } else {
            0
        }
    }

    fn try_acquire(self: &Arc<Self>) -> Option<RequestLease> {
        let mut gate = self.control.gate.lock();
        if gate.traffic != TrafficState::Serving
            || (gate.health != HealthState::Healthy && !gate.health_override)
            || gate.weight == 0
        {
            return None;
        }
        gate.in_flight += 1;
        let generation = gate.generation;
        drop(gate);
        Some(RequestLease {
            instance: Arc::clone(self),
            generation,
            long_lived: false,
        })
    }
}

#[derive(Debug)]
struct RuntimeRoute {
    id: String,
    host: String,
    path_prefix: String,
    backends: Vec<Arc<LiveInstance>>,
    schedule: Mutex<HashMap<String, i64>>,
}

#[derive(Debug, Default)]
struct RuntimeSnapshot {
    routes: Vec<RuntimeRoute>,
    instances: HashMap<String, Arc<LiveInstance>>,
}

/// Immutable routing snapshots plus the small mutable gate held by each backend.
#[derive(Debug)]
pub struct GatewayRuntime {
    snapshot: ArcSwap<RuntimeSnapshot>,
}

impl Default for GatewayRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl GatewayRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            snapshot: ArcSwap::from_pointee(RuntimeSnapshot::default()),
        }
    }

    /// Atomically replaces the route table. Existing request leases keep their old backend alive.
    pub fn publish(&self, config: GatewayConfig, persisted: Vec<PersistedInstanceState>) {
        let persisted: HashMap<_, _> = persisted
            .into_iter()
            .map(|state| (state.id.clone(), state))
            .collect();
        let current = self.snapshot.load();
        let mut instances = HashMap::new();
        let mut routes = Vec::with_capacity(config.routes.len());

        for route in config.routes {
            let mut backends = Vec::with_capacity(route.backends.len());
            for backend in route.backends {
                let live = if let Some(existing) = instances.get(&backend.id) {
                    Arc::clone(existing)
                } else if let Some(existing) = current.instances.get(&backend.id) {
                    if existing.address != backend.address
                        || existing.tls != backend.tls
                        || existing.health_check != backend.health_check
                    {
                        existing.control.gate.lock().health =
                            initial_health(backend.health_check.as_ref());
                    }
                    Arc::new(LiveInstance {
                        address: backend.address,
                        tls: backend.tls,
                        health_check: backend.health_check,
                        control: Arc::clone(&existing.control),
                    })
                } else {
                    let saved = persisted.get(&backend.id);
                    let health = initial_health(backend.health_check.as_ref());
                    Arc::new(LiveInstance {
                        address: backend.address,
                        tls: backend.tls,
                        health_check: backend.health_check,
                        control: Arc::new(InstanceControl {
                            id: backend.id.clone(),
                            gate: Mutex::new(InstanceGate {
                                generation: saved
                                    .map_or(backend.generation, |state| state.generation),
                                weight: saved.map_or(backend.weight, |state| state.weight),
                                traffic: saved.map_or(TrafficState::Serving, |state| state.traffic),
                                health,
                                health_override: saved.is_some_and(|state| state.health_override),
                                in_flight: 0,
                                long_lived_in_flight: 0,
                            }),
                        }),
                    })
                };
                instances.insert(backend.id, Arc::clone(&live));
                backends.push(live);
            }
            routes.push(RuntimeRoute {
                id: route.id,
                host: route.host,
                path_prefix: route.path_prefix,
                backends,
                schedule: Mutex::new(HashMap::new()),
            });
        }

        self.snapshot
            .store(Arc::new(RuntimeSnapshot { routes, instances }));
    }

    /// Selects a backend and acquires an in-flight request lease.
    ///
    /// # Errors
    ///
    /// Returns an error when no route matches or every matched backend is blocked, unhealthy, or
    /// has zero weight.
    pub fn acquire(&self, host: &str, path: &str) -> Result<RequestLease> {
        let snapshot = self.snapshot.load();
        let route = snapshot
            .routes
            .iter()
            .filter_map(|route| route_match_score(route, host, path).map(|score| (route, score)))
            .max_by_key(|(_, score)| *score)
            .map(|(route, _)| route)
            .ok_or_else(|| Error::RouteNotFound {
                host: host.to_owned(),
                path: path.to_owned(),
            })?;

        for _ in 0..=route.backends.len() {
            let weighted: Vec<_> = route
                .backends
                .iter()
                .filter_map(|backend| {
                    let weight = backend.selectable_weight();
                    (weight > 0).then_some((backend, u64::from(weight)))
                })
                .collect();
            let total: u64 = weighted.iter().map(|(_, weight)| weight).sum();
            if total == 0 {
                return Err(Error::NoAvailableBackend(route.id.clone()));
            }
            let total = i64::try_from(total).unwrap_or(i64::MAX);
            let selected = {
                let mut schedule = route.schedule.lock();
                let mut selected: Option<(&Arc<LiveInstance>, i64)> = None;
                for (backend, weight) in weighted {
                    let weight = i64::try_from(weight).unwrap_or(i64::MAX);
                    let current = schedule.entry(backend.control.id.clone()).or_default();
                    *current = current.saturating_add(weight);
                    if selected.is_none_or(|(_, best)| *current > best) {
                        selected = Some((backend, *current));
                    }
                }
                let Some(selected) = selected.map(|(backend, _)| Arc::clone(backend)) else {
                    return Err(Error::NoAvailableBackend(route.id.clone()));
                };
                let current = schedule.entry(selected.control.id.clone()).or_default();
                *current = current.saturating_sub(total);
                selected
            };
            if let Some(lease) = selected.try_acquire() {
                return Ok(lease);
            }
        }

        Err(Error::NoAvailableBackend(route.id.clone()))
    }

    /// Publishes the latest observed health without changing the operator-controlled traffic
    /// state.
    ///
    /// # Errors
    ///
    /// Returns an error when the instance does not exist in the current snapshot.
    pub fn report_health(&self, id: &str, health: HealthState) -> Result<InstanceState> {
        let instance = self.instance(id)?;
        instance.control.gate.lock().health = health;
        Ok(instance.state())
    }

    /// Returns the active probe inputs from one immutable routing snapshot.
    #[must_use]
    pub fn health_targets(&self) -> Vec<HealthTarget> {
        let snapshot = self.snapshot.load();
        snapshot
            .instances
            .values()
            .filter_map(|instance| {
                instance.health_check.clone().map(|check| HealthTarget {
                    id: instance.control.id.clone(),
                    address: instance.address,
                    tls: instance.tls.clone(),
                    check,
                })
            })
            .collect()
    }

    pub(crate) fn instance_state(&self, id: &str) -> Result<InstanceState> {
        self.instance(id).map(|instance| instance.state())
    }

    pub(crate) fn instance_states(&self) -> Vec<InstanceState> {
        let snapshot = self.snapshot.load();
        let mut states = snapshot
            .instances
            .values()
            .map(|instance| instance.state())
            .collect::<Vec<_>>();
        states.sort_by(|left, right| left.id.cmp(&right.id));
        states
    }

    pub(crate) fn describe_route(
        &self,
        host: &str,
        path: &str,
    ) -> Option<(String, Vec<InstanceState>)> {
        let snapshot = self.snapshot.load();
        snapshot
            .routes
            .iter()
            .filter_map(|route| route_match_score(route, host, path).map(|score| (route, score)))
            .max_by_key(|(_, score)| *score)
            .map(|(route, _)| route)
            .map(|route| {
                (
                    route.id.clone(),
                    route
                        .backends
                        .iter()
                        .map(|instance| instance.state())
                        .collect(),
                )
            })
    }

    pub(crate) fn drain(&self, id: &str, force: bool) -> Result<InstanceState> {
        let instance = self.instance(id)?;
        if !force {
            let snapshot = self.snapshot.load();
            for route in &snapshot.routes {
                if !route
                    .backends
                    .iter()
                    .any(|backend| backend.control.id == id)
                {
                    continue;
                }
                let has_alternative = route
                    .backends
                    .iter()
                    .any(|backend| backend.control.id != id && backend.selectable_weight() > 0);
                if !has_alternative {
                    return Err(Error::LastAvailableBackend {
                        instance_id: id.to_owned(),
                        route_id: route.id.clone(),
                    });
                }
            }
        }
        let mut gate = instance.control.gate.lock();
        gate.traffic = if gate.in_flight == 0 {
            TrafficState::Drained
        } else {
            TrafficState::Draining
        };
        drop(gate);
        Ok(instance.state())
    }

    pub(crate) fn rejoin(
        &self,
        id: &str,
        generation: u64,
        weight: u32,
        force: bool,
    ) -> Result<InstanceState> {
        let instance = self.instance(id)?;
        let mut gate = instance.control.gate.lock();
        if gate.in_flight > 0
            || !matches!(gate.traffic, TrafficState::Drained | TrafficState::Disabled)
        {
            return Err(Error::InvalidState(format!(
                "instance {id} must be fully drained or disabled before rejoin"
            )));
        }
        if !force && gate.health != HealthState::Healthy {
            return Err(Error::InvalidState(format!("instance {id} is not healthy")));
        }
        if generation <= gate.generation {
            return Err(Error::InvalidState(format!(
                "generation must increase beyond {}",
                gate.generation
            )));
        }
        gate.generation = generation;
        gate.weight = weight;
        gate.traffic = TrafficState::Serving;
        gate.health_override = force;
        drop(gate);
        Ok(instance.state())
    }

    pub(crate) fn set_weight(&self, id: &str, weight: u32) -> Result<InstanceState> {
        let instance = self.instance(id)?;
        instance.control.gate.lock().weight = weight;
        Ok(instance.state())
    }

    pub(crate) fn disable(&self, id: &str) -> Result<InstanceState> {
        let instance = self.instance(id)?;
        instance.control.gate.lock().traffic = TrafficState::Disabled;
        Ok(instance.state())
    }

    pub(crate) fn restore_control_state(&self, state: &InstanceState) -> Result<()> {
        let instance = self.instance(&state.id)?;
        let mut gate = instance.control.gate.lock();
        gate.generation = state.generation;
        gate.weight = state.weight;
        gate.traffic = state.traffic;
        gate.health_override = state.health_override;
        Ok(())
    }

    fn instance(&self, id: &str) -> Result<Arc<LiveInstance>> {
        self.snapshot
            .load()
            .instances
            .get(id)
            .cloned()
            .ok_or_else(|| Error::InstanceNotFound(id.to_owned()))
    }
}

fn route_match_score(route: &RuntimeRoute, host: &str, path: &str) -> Option<(usize, u8, usize)> {
    let (host_kind, host_length) = host_specificity(&route.host, host)?;
    let path_matches = path.starts_with(&route.path_prefix)
        && (route.path_prefix == "/"
            || route.path_prefix.ends_with('/')
            || path.len() == route.path_prefix.len()
            || path.as_bytes().get(route.path_prefix.len()) == Some(&b'/'));
    path_matches.then_some((route.path_prefix.len(), host_kind, host_length))
}

fn host_specificity(pattern: &str, host: &str) -> Option<(u8, usize)> {
    if pattern.eq_ignore_ascii_case(host) {
        return Some((2, pattern.len()));
    }
    let suffix = pattern.strip_prefix("*.")?;
    let prefix = host.get(..host.len().checked_sub(suffix.len() + 1)?)?;
    (host.as_bytes().get(prefix.len()) == Some(&b'.')
        && host[prefix.len() + 1..].eq_ignore_ascii_case(suffix)
        && !prefix.is_empty()
        && !prefix.contains('.'))
    .then_some((1, suffix.len()))
}

fn initial_health(check: Option<&HealthCheckConfig>) -> HealthState {
    if check.is_some() {
        HealthState::Unknown
    } else {
        HealthState::Healthy
    }
}

/// A request's exclusive claim on one backend. Dropping it completes the request.
#[derive(Debug)]
pub struct RequestLease {
    instance: Arc<LiveInstance>,
    generation: u64,
    long_lived: bool,
}

impl RequestLease {
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance.control.id
    }

    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.instance.address
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn upstream_tls(&self) -> Option<&UpstreamTlsConfig> {
        self.instance.tls.as_ref()
    }

    /// Moves this request from the ordinary bucket into the long-lived bucket.
    pub fn mark_long_lived(&mut self) {
        if self.long_lived {
            return;
        }
        self.long_lived = true;
        self.instance.control.gate.lock().long_lived_in_flight += 1;
    }
}

impl Drop for RequestLease {
    fn drop(&mut self) {
        let mut gate = self.instance.control.gate.lock();
        gate.in_flight = gate.in_flight.saturating_sub(1);
        if self.long_lived {
            gate.long_lived_in_flight = gate.long_lived_in_flight.saturating_sub(1);
        }
        if gate.in_flight == 0 && gate.traffic == TrafficState::Draining {
            gate.traffic = TrafficState::Drained;
        }
    }
}
