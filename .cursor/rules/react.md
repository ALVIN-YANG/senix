# Senix React Frontend Rules

1. **Framework & State**:
   - Use React 18+ functional components with Hooks.
   - Use `Zustand` for global state management (e.g., `stores/auth.js`).

2. **Styling & UI**:
   - Follow the 2026 Modern Premium Design (Deep Space Dark Theme).
   - Use `Arco Design` components.
   - Custom CSS should prioritize CSS variables defined in `index.css`.
   - Always implement responsive design and hover states (e.g., transform, box-shadow) for interactive elements.

3. **API Integration**:
   - Use Axios with global interceptors.
   - The `Authorization` header must be injected via `useAuthStore.getState().initAuth()`.
   - Handle 401 Unauthorized responses by clearing the token and redirecting to `/login`.

4. **Routing**:
   - Use `react-router-dom` v6.
   - Protect private routes using the `PrivateRoute` component in `App.jsx`.