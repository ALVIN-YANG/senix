import { useState } from 'react'
import {
  Card,
  Form,
  Input,
  Button,
  Message,
  Tabs,
  Switch
} from '@arco-design/web-react'
import { IconSave } from '@arco-design/web-react/icon'
import './Settings.css'

const FormItem = Form.Item
const TabPane = Tabs.TabPane

function Settings() {
  const [systemForm] = Form.useForm()
  const [userForm] = Form.useForm()

  const handleSaveSystem = async (values) => {
    Message.success('系统设置已保存')
  }

  const handleSaveUser = async (values) => {
    Message.success('个人信息已更新')
  }

  return (
    <div className="settings-page">
      <div className="page-header">
        <h1>系统设置</h1>
      </div>

      <Card bordered={false}>
        <Tabs defaultActiveTab="system">
          <TabPane title="系统配置" key="system">
            <Form
              form={systemForm}
              onSubmit={handleSaveSystem}
              layout="vertical"
              style={{ maxWidth: 600 }}
            >
              <FormItem label="系统名称" field="system_name" initialValue="Senix Gateway">
                <Input placeholder="系统名称" />
              </FormItem>

              <FormItem label="数据目录" field="data_dir" initialValue="/var/lib/senix">
                <Input placeholder="数据存储目录" />
              </FormItem>

              <FormItem label="Nginx 配置目录" field="nginx_dir" initialValue="/etc/nginx/conf.d">
                <Input placeholder="Nginx 配置目录" />
              </FormItem>

              <FormItem label="自动续期证书" field="auto_renew" triggerPropName="checked" initialValue={true}>
                <Switch />
              </FormItem>

              <FormItem label="自动重载 Nginx" field="auto_reload" triggerPropName="checked" initialValue={true}>
                <Switch />
              </FormItem>

              <FormItem>
                <Button type="primary" htmlType="submit" icon={<IconSave />}>
                  保存配置
                </Button>
              </FormItem>
            </Form>
          </TabPane>

          <TabPane title="个人设置" key="profile">
            <Form
              form={userForm}
              onSubmit={handleSaveUser}
              layout="vertical"
              style={{ maxWidth: 600 }}
            >
              <FormItem label="用户名" field="username" initialValue="admin">
                <Input disabled />
              </FormItem>

              <FormItem label="当前密码" field="current_password">
                <Input.Password placeholder="输入当前密码" />
              </FormItem>

              <FormItem label="新密码" field="new_password">
                <Input.Password placeholder="输入新密码" />
              </FormItem>

              <FormItem label="确认新密码" field="confirm_password">
                <Input.Password placeholder="确认新密码" />
              </FormItem>

              <FormItem>
                <Button type="primary" htmlType="submit" icon={<IconSave />}>
                  更新密码
                </Button>
              </FormItem>
            </Form>
          </TabPane>
        </Tabs>
      </Card>
    </div>
  )
}

export default Settings
