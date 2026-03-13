import { useEffect, useState } from 'react'
import { Card, Grid, Statistic, Button, Message } from '@arco-design/web-react'
import {
  IconApps,
  IconCheckCircle,
  IconLock,
  IconExclamationCircle,
  IconRefresh,
  IconPlus
} from '@arco-design/web-react/icon'
import { useNavigate } from 'react-router-dom'
import axios from 'axios'
import './Dashboard.css'

const { Row, Col } = Grid

function Dashboard() {
  const navigate = useNavigate()
  const [siteStats, setSiteStats] = useState({})
  const [certStats, setCertStats] = useState({})
  const [loading, setLoading] = useState(false)

  const fetchStats = async () => {
    try {
      const [siteRes, certRes] = await Promise.all([
        axios.get('/api/sites/stats'),
        axios.get('/api/certs/stats')
      ])

      if (siteRes.data.code === 200) {
        setSiteStats(siteRes.data.data)
      }
      if (certRes.data.code === 200) {
        setCertStats(certRes.data.data)
      }
    } catch (error) {
      console.error('Failed to fetch stats:', error)
    }
  }

  const handleGenerateAll = async () => {
    setLoading(true)
    try {
      const res = await axios.post('/api/sites/generate-all')
      if (res.data.code === 200) {
        Message.success('配置生成成功')
      } else {
        Message.error(res.data.message)
      }
    } catch {
      Message.error('配置生成失败')
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    fetchStats()
  }, [])

  const getModePercentage = (mode) => {
    const total = siteStats.total || 0
    if (total === 0) return 0
    const count = siteStats[mode] || 0
    return Math.round((count / total) * 100)
  }

  return (
    <div className="dashboard">
      <div className="page-header">
        <h1>系统概览</h1>
      </div>

      {/* 统计卡片 */}
      <Row gutter={[16, 16]} className="stats-row">
        <Col span={6}>
          <Card className="stat-card" bordered={false}>
            <div className="stat-content">
              <div className="stat-icon blue">
                <IconApps style={{ fontSize: 24 }} />
              </div>
              <Statistic
                title="站点总数"
                value={siteStats.total || 0}
                valueStyle={{ fontSize: 28, fontWeight: 600 }}
              />
            </div>
          </Card>
        </Col>
        <Col span={6}>
          <Card className="stat-card" bordered={false}>
            <div className="stat-content">
              <div className="stat-icon green">
                <IconCheckCircle style={{ fontSize: 24 }} />
              </div>
              <Statistic
                title="运行中"
                value={siteStats.active || 0}
                valueStyle={{ fontSize: 28, fontWeight: 600, color: '#34d399' }}
              />
            </div>
          </Card>
        </Col>
        <Col span={6}>
          <Card className="stat-card" bordered={false}>
            <div className="stat-content">
              <div className="stat-icon purple">
                <IconLock style={{ fontSize: 24 }} />
              </div>
              <Statistic
                title="证书总数"
                value={certStats.total || 0}
                valueStyle={{ fontSize: 28, fontWeight: 600 }}
              />
            </div>
          </Card>
        </Col>
        <Col span={6}>
          <Card className="stat-card" bordered={false}>
            <div className="stat-content">
              <div className="stat-icon orange">
                <IconExclamationCircle style={{ fontSize: 24 }} />
              </div>
              <Statistic
                title="即将过期"
                value={certStats.expiring_soon || 0}
                valueStyle={{ fontSize: 28, fontWeight: 600, color: '#fbbf24' }}
              />
            </div>
          </Card>
        </Col>
      </Row>

      {/* 工作模式分布和证书状态 */}
      <Row gutter={[16, 16]} className="charts-row">
        <Col span={12}>
          <Card title="工作模式分布" bordered={false}>
            <div className="mode-stats">
              <div className="mode-item">
                <div className="mode-info">
                  <span className="mode-name">独立代理模式</span>
                  <span className="mode-value">{siteStats.standalone || 0}</span>
                </div>
                <div className="progress-bar">
                  <div
                    className="progress-fill blue"
                    style={{ width: `${getModePercentage('standalone')}%` }}
                  />
                </div>
              </div>
              <div className="mode-item">
                <div className="mode-info">
                  <span className="mode-name">证书管理模式</span>
                  <span className="mode-value">{siteStats.cert_only || 0}</span>
                </div>
                <div className="progress-bar">
                  <div
                    className="progress-fill green"
                    style={{ width: `${getModePercentage('cert_only')}%` }}
                  />
                </div>
              </div>
              <div className="mode-item">
                <div className="mode-info">
                  <span className="mode-name">配置生成模式</span>
                  <span className="mode-value">{siteStats.config_only || 0}</span>
                </div>
                <div className="progress-bar">
                  <div
                    className="progress-fill orange"
                    style={{ width: `${getModePercentage('config_only')}%` }}
                  />
                </div>
              </div>
            </div>
          </Card>
        </Col>
        <Col span={12}>
          <Card title="SSL 证书状态" bordered={false}>
            <div className="cert-stats">
              <div className="cert-stat-item">
                <div className="cert-value success">{certStats.active || 0}</div>
                <div className="cert-label">有效证书</div>
              </div>
              <div className="cert-stat-item">
                <div className="cert-value warning">{certStats.expiring_soon || 0}</div>
                <div className="cert-label">即将过期</div>
              </div>
              <div className="cert-stat-item">
                <div className="cert-value error">{certStats.expired || 0}</div>
                <div className="cert-label">已过期</div>
              </div>
            </div>
          </Card>
        </Col>
      </Row>

      {/* 快捷操作 */}
      <Card title="快捷操作" bordered={false} className="quick-actions">
        <div className="actions">
          <Button type="primary" icon={<IconPlus />} onClick={() => navigate('/sites')}>
            添加站点
          </Button>
          <Button type="primary" status="success" icon={<IconLock />} onClick={() => navigate('/certificates')}>
            申请证书
          </Button>
          <Button icon={<IconRefresh />} loading={loading} onClick={handleGenerateAll}>
            生成所有配置
          </Button>
        </div>
      </Card>
    </div>
  )
}

export default Dashboard
