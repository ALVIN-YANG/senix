import { useState } from 'react'
import {
  Card,
  Tabs,
  Form,
  Input,
  Button,
  Switch,
  InputNumber,
  Message,
  Table,
  Space
} from '@arco-design/web-react'
import { IconSave, IconPlus, IconDelete } from '@arco-design/web-react/icon'
import './Security.css'

const FormItem = Form.Item
const TabPane = Tabs.TabPane

function Security() {
  const [wafForm] = Form.useForm()
  const [rateLimitForm] = Form.useForm()
  const [ipForm] = Form.useForm()
  const [blacklist, setBlacklist] = useState([])

  const handleSaveWAF = async (values) => {
    Message.success('WAF 配置已保存')
  }

  const handleSaveRateLimit = async (values) => {
    Message.success('限流配置已保存')
  }

  const handleAddIP = (values) => {
    setBlacklist([...blacklist, { ip: values.ip, remark: values.remark }])
    ipForm.resetFields()
  }

  const handleDeleteIP = (ip) => {
    setBlacklist(blacklist.filter(item => item.ip !== ip))
  }

  const ipColumns = [
    {
      title: 'IP 地址',
      dataIndex: 'ip'
    },
    {
      title: '备注',
      dataIndex: 'remark'
    },
    {
      title: '操作',
      render: (_, record) => (
        <Button
          type="text"
          status="danger"
          icon={<IconDelete />}
          onClick={() => handleDeleteIP(record.ip)}
        >
          删除
        </Button>
      )
    }
  ]

  return (
    <div className="security-page">
      <div className="page-header">
        <h1>安全策略</h1>
      </div>

      <Card bordered={false}>
        <Tabs defaultActiveTab="waf">
          <TabPane title="WAF 防护" key="waf">
            <Form
              form={wafForm}
              onSubmit={handleSaveWAF}
              layout="vertical"
              style={{ maxWidth: 600 }}
            >
              <FormItem label="启用 WAF" field="enabled" triggerPropName="checked">
                <Switch />
              </FormItem>

              <FormItem label="防护模式" field="mode" initialValue="detection">
                <Input.Group>
                  <Button type="primary">检测模式</Button>
                  <Button>拦截模式</Button>
                </Input.Group>
              </FormItem>

              <FormItem label="SQL 注入防护" field="sql_injection" triggerPropName="checked" initialValue={true}>
                <Switch />
              </FormItem>

              <FormItem label="XSS 防护" field="xss" triggerPropName="checked" initialValue={true}>
                <Switch />
              </FormItem>

              <FormItem label="命令注入防护" field="command_injection" triggerPropName="checked" initialValue={true}>
                <Switch />
              </FormItem>

              <FormItem>
                <Button type="primary" htmlType="submit" icon={<IconSave />}>
                  保存配置
                </Button>
              </FormItem>
            </Form>
          </TabPane>

          <TabPane title="限流策略" key="ratelimit">
            <Form
              form={rateLimitForm}
              onSubmit={handleSaveRateLimit}
              layout="vertical"
              style={{ maxWidth: 600 }}
            >
              <FormItem label="启用限流" field="enabled" triggerPropName="checked">
                <Switch />
              </FormItem>

              <FormItem label="每秒请求数 (req/s)" field="requests_per_second" initialValue={100}>
                <InputNumber min={1} max={10000} style={{ width: 200 }} />
              </FormItem>

              <FormItem label="突发流量限制" field="burst" initialValue={200}>
                <InputNumber min={1} max={50000} style={{ width: 200 }} />
              </FormItem>

              <FormItem>
                <Button type="primary" htmlType="submit" icon={<IconSave />}>
                  保存配置
                </Button>
              </FormItem>
            </Form>
          </TabPane>

          <TabPane title="IP 黑名单" key="blacklist">
            <div className="blacklist-section">
              <Form
                form={ipForm}
                onSubmit={handleAddIP}
                layout="inline"
                style={{ marginBottom: 24 }}
              >
                <FormItem field="ip" rules={[{ required: true }]}>
                  <Input placeholder="IP 地址或 CIDR" style={{ width: 200 }} />
                </FormItem>
                <FormItem field="remark">
                  <Input placeholder="备注" style={{ width: 200 }} />
                </FormItem>
                <FormItem>
                  <Button type="primary" icon={<IconPlus />} htmlType="submit">
                    添加
                  </Button>
                </FormItem>
              </Form>

              <Table
                columns={ipColumns}
                data={blacklist}
                rowKey="ip"
                pagination={false}
              />
            </div>
          </TabPane>
        </Tabs>
      </Card>
    </div>
  )
}

export default Security
