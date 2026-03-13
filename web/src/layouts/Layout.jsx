import { useState } from 'react'
import { Outlet, useLocation, useNavigate } from 'react-router-dom'
import {
  Layout as ArcoLayout,
  Menu,
  Button,
  Dropdown,
  Avatar,
  Message
} from '@arco-design/web-react'
import {
  IconDashboard,
  IconApps,
  IconSafe,
  IconSettings,
  IconLock,
  IconUser,
  IconDown,
  IconMenuFold,
  IconMenuUnfold
} from '@arco-design/web-react/icon'
import { useAuthStore } from '../stores/auth'
import './Layout.css'

const { Sider, Header, Content } = ArcoLayout
const MenuItem = Menu.Item
const SubMenu = Menu.SubMenu

const menuItems = [
  {
    key: '/dashboard',
    icon: <IconDashboard />,
    title: '概览'
  },
  {
    key: '/sites',
    icon: <IconApps />,
    title: '站点管理'
  },
  {
    key: '/certificates',
    icon: <IconLock />,
    title: '证书管理'
  },
  {
    key: '/security',
    icon: <IconSafe />,
    title: '安全策略'
  },
  {
    key: '/settings',
    icon: <IconSettings />,
    title: '系统设置'
  }
]

function Layout() {
  const [collapsed, setCollapsed] = useState(false)
  const location = useLocation()
  const navigate = useNavigate()
  const { user, logout } = useAuthStore()

  const handleMenuClick = (key) => {
    navigate(key)
  }

  const handleLogout = async () => {
    try {
      await logout()
      Message.success('已退出登录')
      navigate('/login')
    } catch {
      Message.error('退出失败')
    }
  }

  const dropdownMenu = (
    <Menu>
      <Menu.Item key="profile" onClick={() => navigate('/settings')}>
        <IconUser style={{ marginRight: 8 }} />
        个人设置
      </Menu.Item>
      <Menu.Item key="logout" onClick={handleLogout}>
        <IconSettings style={{ marginRight: 8 }} />
        退出登录
      </Menu.Item>
    </Menu>
  )

  return (
    <ArcoLayout className="layout">
      <Sider
        collapsed={collapsed}
        onCollapse={setCollapsed}
        collapsible
        trigger={null}
        width={240}
        className="sider"
      >
        <div className="logo">
          <div className="logo-icon">
            <IconSafe style={{ fontSize: 28, color: '#fff' }} />
          </div>
          {!collapsed && <span className="logo-text">Senix Gateway</span>}
        </div>
        
        <Menu
          theme="light"
          selectedKeys={[location.pathname]}
          onClickMenuItem={handleMenuClick}
          style={{ width: '100%' }}
        >
          {menuItems.map((item) => (
            <MenuItem key={item.key}>
              {item.icon}
              {item.title}
            </MenuItem>
          ))}
        </Menu>
      </Sider>

      <ArcoLayout>
        <Header className="header">
          <Button
            type="text"
            icon={collapsed ? <IconMenuUnfold /> : <IconMenuFold />}
            onClick={() => setCollapsed(!collapsed)}
            style={{ marginRight: 16 }}
          />
          
          <div className="header-right">
            <Dropdown droplist={dropdownMenu} position="br">
              <div className="user-info">
                <Avatar size={32} style={{ backgroundColor: '#3370ff' }}>
                  {user?.username?.[0]?.toUpperCase() || 'A'}
                </Avatar>
                <span className="username">{user?.username || 'Admin'}</span>
                <IconDown style={{ fontSize: 12, marginLeft: 4 }} />
              </div>
            </Dropdown>
          </div>
        </Header>

        <Content className="content">
          <Outlet />
        </Content>
      </ArcoLayout>
    </ArcoLayout>
  )
}

export default Layout
