import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import {
  Card,
  Table,
  Button,
  Tag,
  Space,
  Modal,
  Form,
  Input,
  Select,
  Switch,
  Message,
  Popconfirm
} from '@arco-design/web-react'
import {
  IconPlus,
  IconEdit,
  IconDelete,
  IconPlay,
  IconPause,
  IconEye
} from '@arco-design/web-react/icon'
import axios from 'axios'
import './Sites.css'

const FormItem = Form.Item
const Option = Select.Option

function Sites() {
  const navigate = useNavigate()
  const [sites, setSites] = useState([])
  const [loading, setLoading] = useState(false)
  const [visible, setVisible] = useState(false)
  const [editingSite, setEditingSite] = useState(null)
  const [form] = Form.useForm()
  const [pagination, setPagination] = useState({
    current: 1,
    pageSize: 10,
    total: 0
  })

  const fetchSites = async (page = 1) => {
    setLoading(true)
    try {
      const res = await axios.get(`/api/sites?page=${page}&page_size=${pagination.pageSize}`)
      if (res.data.code === 200) {
        setSites(res.data.data.list)
        setPagination({
          ...pagination,
          current: page,
          total: res.data.data.total
        })
      }
    } catch (error) {
      Message.error('获取站点列表失败')
    } finally {
      setLoading(false)
    }
  }

  const handleEnable = async (id) => {
    try {
      const res = await axios.post(`/api/sites/${id}/enable`)
      if (res.data.code === 200) {
        Message.success('站点已启用')
        fetchSites(pagination.current)
      } else {
        Message.error(res.data.message)
      }
    } catch {
      Message.error('启用失败')
    }
  }

  const handleDisable = async (id) => {
    try {
      const res = await axios.post(`/api/sites/${id}/disable`)
      if (res.data.code === 200) {
        Message.success('站点已禁用')
        fetchSites(pagination.current)
      }
    } catch {
      Message.error('禁用失败')
    }
  }

  const handleDelete = async (id) => {
    try {
      const res = await axios.delete(`/api/sites/${id}`)
      if (res.data.code === 200) {
        Message.success('站点已删除')
        fetchSites(pagination.current)
      }
    } catch {
      Message.error('删除失败')
    }
  }

  const handleSubmit = async (values) => {
    try {
      const url = editingSite ? `/api/sites/${editingSite.id}` : '/api/sites'
      const method = editingSite ? 'put' : 'post'
      const res = await axios[method](url, values)
      
      if (res.data.code === 200) {
        Message.success(editingSite ? '站点已更新' : '站点已创建')
        setVisible(false)
        fetchSites(pagination.current)
      } else {
        Message.error(res.data.message)
      }
    } catch (error) {
      Message.error(error.response?.data?.message || '操作失败')
    }
  }

  const openModal = (site = null) => {
    setEditingSite(site)
    if (site) {
      form.setFieldsValue({
        name: site.name,
        domain: site.domain,
        work_mode: site.work_mode,
        port: site.port,
        ssl_enabled: site.ssl_enabled,
        upstream: site.upstream,
        enable_waf: site.enable_waf,
        enable_rate_limit: site.enable_rate_limit,
        description: site.description
      })
    } else {
      form.resetFields()
    }
    setVisible(true)
  }

  useEffect(() => {
    fetchSites()
  }, [])

  const columns = [
    {
      title: '名称',
      dataIndex: 'name',
      render: (text, record) => (
        <div>
          <div className="site-name">{text}</div>
          <div className="site-domain">{record.domain}</div>
        </div>
      )
    },
    {
      title: '工作模式',
      dataIndex: 'work_mode',
      render: (mode) => {
        const modeMap = {
          standalone: { label: '独立代理', color: 'blue' },
          cert_only: { label: '证书管理', color: 'green' },
          config_only: { label: '配置生成', color: 'orange' }
        }
        const { label, color } = modeMap[mode] || { label: mode, color: 'default' }
        return <Tag color={color}>{label}</Tag>
      }
    },
    {
      title: 'SSL',
      dataIndex: 'ssl_enabled',
      render: (enabled) => (
        <Tag color={enabled ? 'green' : 'default'}>
          {enabled ? '已启用' : '未启用'}
        </Tag>
      )
    },
    {
      title: '状态',
      dataIndex: 'status',
      render: (status) => (
        <Tag color={status === 'active' ? 'green' : 'default'}>
          {status === 'active' ? '运行中' : '已停止'}
        </Tag>
      )
    },
    {
      title: '操作',
      render: (_, record) => (
        <Space>
          {record.status === 'active' ? (
            <Button
              type="text"
              icon={<IconPause />}
              onClick={() => handleDisable(record.id)}
            >
              禁用
            </Button>
          ) : (
            <Button
              type="text"
              icon={<IconPlay />}
              onClick={() => handleEnable(record.id)}
            >
              启用
            </Button>
          )}
          <Button
            type="text"
            icon={<IconEdit />}
            onClick={() => openModal(record)}
          >
            编辑
          </Button>
          <Popconfirm
            title="确认删除"
            content="删除后无法恢复，是否继续？"
            onOk={() => handleDelete(record.id)}
          >
            <Button type="text" status="danger" icon={<IconDelete />}>
              删除
            </Button>
          </Popconfirm>
        </Space>
      )
    }
  ]

  return (
    <div className="sites-page">
      <div className="page-header">
        <h1>站点管理</h1>
        <Button type="primary" icon={<IconPlus />} onClick={() => openModal()}>
          添加站点
        </Button>
      </div>

      <Card bordered={false}>
        <Table
          columns={columns}
          data={sites}
          loading={loading}
          pagination={{
            ...pagination,
            onChange: fetchSites
          }}
          rowKey="id"
        />
      </Card>

      <Modal
        title={editingSite ? '编辑站点' : '添加站点'}
        visible={visible}
        onOk={() => form.submit()}
        onCancel={() => setVisible(false)}
        autoFocus={false}
        style={{ width: 600 }}
      >
        <Form
          form={form}
          onSubmit={handleSubmit}
          layout="vertical"
          autoComplete="off"
        >
          <FormItem label="站点名称" field="name" rules={[{ required: true }]}>
            <Input placeholder="输入站点名称" />
          </FormItem>

          <FormItem label="域名" field="domain" rules={[{ required: true }]}>
            <Input placeholder="example.com" />
          </FormItem>

          <FormItem label="工作模式" field="work_mode" rules={[{ required: true }]}>
            <Select placeholder="选择工作模式">
              <Option value="standalone">独立代理模式</Option>
              <Option value="cert_only">证书管理模式</Option>
              <Option value="config_only">配置生成模式</Option>
            </Select>
          </FormItem>

          <FormItem label="端口" field="port" initialValue={80}>
            <Input type="number" placeholder="80" />
          </FormItem>

          <FormItem label="后端服务" field="upstream">
            <Input placeholder="http://localhost:8080" />
          </FormItem>

          <FormItem label="启用 SSL" field="ssl_enabled" triggerPropName="checked">
            <Switch />
          </FormItem>

          <FormItem label="启用 WAF" field="enable_waf" triggerPropName="checked">
            <Switch />
          </FormItem>

          <FormItem label="启用限流" field="enable_rate_limit" triggerPropName="checked">
            <Switch />
          </FormItem>

          <FormItem label="描述" field="description">
            <Input.TextArea placeholder="站点描述（可选）" rows={3} />
          </FormItem>
        </Form>
      </Modal>
    </div>
  )
}

export default Sites
