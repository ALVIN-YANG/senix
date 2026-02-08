import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import {
  Form,
  Input,
  Button,
  Card,
  Message
} from '@arco-design/web-react'
import { IconSafe, IconUser, IconLock } from '@arco-design/web-react/icon'
import { useAuthStore } from '../stores/auth'
import './Login.css'

const FormItem = Form.Item

function Login() {
  const [loading, setLoading] = useState(false)
  const [form] = Form.useForm()
  const navigate = useNavigate()
  const login = useAuthStore((state) => state.login)

  const handleSubmit = async (values) => {
    setLoading(true)
    try {
      const result = await login(values)
      if (result.success) {
        Message.success('登录成功')
        navigate('/')
      } else {
        Message.error(result.message)
      }
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="login-container">
      <Card className="login-card" bordered={false}>
        <div className="login-header">
          <div className="login-logo">
            <IconSafe style={{ fontSize: 48, color: '#3370ff' }} />
          </div>
          <h1>Senix Gateway</h1>
          <p>高性能网关管理平台</p>
        </div>

        <Form
          form={form}
          onSubmit={handleSubmit}
          autoComplete="off"
          layout="vertical"
        >
          <FormItem
            field="username"
            rules={[{ required: true, message: '请输入用户名' }]}
          >
            <Input
              prefix={<IconUser />}
              placeholder="用户名"
              size="large"
            />
          </FormItem>

          <FormItem
            field="password"
            rules={[{ required: true, message: '请输入密码' }]}
          >
            <Input.Password
              prefix={<IconLock />}
              placeholder="密码"
              size="large"
            />
          </FormItem>

          <FormItem>
            <Button
              type="primary"
              htmlType="submit"
              long
              size="large"
              loading={loading}
            >
              登录
            </Button>
          </FormItem>
        </Form>

        <div className="login-tips">
          <p>默认账号: admin / admin</p>
        </div>
      </Card>
    </div>
  )
}

export default Login
