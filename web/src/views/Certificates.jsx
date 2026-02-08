import { useEffect, useState } from 'react'
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
  Message,
  Popconfirm
} from '@arco-design/web-react'
import {
  IconPlus,
  IconRefresh,
  IconDelete,
  IconEye
} from '@arco-design/web-react/icon'
import axios from 'axios'
import './Certificates.css'

const FormItem = Form.Item
const Option = Select.Option

function Certificates() {
  const [certs, setCerts] = useState([])
  const [loading, setLoading] = useState(false)
  const [visible, setVisible] = useState(false)
  const [form] = Form.useForm()
  const [pagination, setPagination] = useState({
    current: 1,
    pageSize: 10,
    total: 0
  })

  const fetchCerts = async (page = 1) => {
    setLoading(true)
    try {
      const res = await axios.get(`/api/certs?page=${page}&page_size=${pagination.pageSize}`)
      if (res.data.code === 200) {
        setCerts(res.data.data.list)
        setPagination({
          ...pagination,
          current: page,
          total: res.data.data.total
        })
      }
    } catch (error) {
      Message.error('获取证书列表失败')
    } finally {
      setLoading(false)
    }
  }

  const handleRenew = async (id) => {
    try {
      const res = await axios.post(`/api/certs/${id}/renew`)
      if (res.data.code === 200) {
        Message.success('证书续期成功')
        fetchCerts(pagination.current)
      } else {
        Message.error(res.data.message)
      }
    } catch {
      Message.error('续期失败')
    }
  }

  const handleDelete = async (id) => {
    try {
      const res = await axios.delete(`/api/certs/${id}`)
      if (res.data.code === 200) {
        Message.success('证书已删除')
        fetchCerts(pagination.current)
      }
    } catch {
      Message.error('删除失败')
    }
  }

  const handleSubmit = async (values) => {
    try {
      const res = await axios.post('/api/certs', values)
      if (res.data.code === 200) {
        Message.success('证书申请已提交')
        setVisible(false)
        form.resetFields()
        fetchCerts()
      } else {
        Message.error(res.data.message)
      }
    } catch (error) {
      Message.error(error.response?.data?.message || '申请失败')
    }
  }

  useEffect(() => {
    fetchCerts()
  }, [])

  const getStatusTag = (cert) => {
    if (cert.is_revoked) {
      return <Tag color="red">已吊销</Tag>
    }
    if (cert.expires_at) {
      const days = Math.ceil((new Date(cert.expires_at) - new Date()) / (1000 * 60 * 60 * 24))
      if (days < 0) {
        return <Tag color="red">已过期</Tag>
      }
      if (days < 30) {
        return <Tag color="orange">即将过期 ({days}天)</Tag>
      }
    }
    return <Tag color="green">有效</Tag>
  }

  const columns = [
    {
      title: '域名',
      dataIndex: 'domain',
      render: (text) => <span className="cert-domain">{text}</span>
    },
    {
      title: '颁发者',
      dataIndex: 'issuer',
      render: (text) => text || 'Let\'s Encrypt'
    },
    {
      title: '类型',
      dataIndex: 'cert_type',
      render: (type) => {
        const typeMap = {
          letsencrypt: { label: 'Let\'s Encrypt', color: 'blue' },
          custom: { label: '自定义', color: 'purple' }
        }
        const { label, color } = typeMap[type] || { label: type, color: 'default' }
        return <Tag color={color}>{label}</Tag>
      }
    },
    {
      title: '过期时间',
      dataIndex: 'expires_at',
      render: (date) => date ? new Date(date).toLocaleDateString('zh-CN') : '-'
    },
    {
      title: '状态',
      render: (_, record) => getStatusTag(record)
    },
    {
      title: '操作',
      render: (_, record) => (
        <Space>
          <Button
            type="text"
            icon={<IconRefresh />}
            onClick={() => handleRenew(record.id)}
            disabled={record.cert_type !== 'letsencrypt'}
          >
            续期
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
    <div className="certs-page">
      <div className="page-header">
        <h1>证书管理</h1>
        <Button type="primary" icon={<IconPlus />} onClick={() => setVisible(true)}>
          申请证书
        </Button>
      </div>

      <Card bordered={false}>
        <Table
          columns={columns}
          data={certs}
          loading={loading}
          pagination={{
            ...pagination,
            onChange: fetchCerts
          }}
          rowKey="id"
        />
      </Card>

      <Modal
        title="申请证书"
        visible={visible}
        onOk={() => form.submit()}
        onCancel={() => setVisible(false)}
        autoFocus={false}
        style={{ width: 500 }}
      >
        <Form
          form={form}
          onSubmit={handleSubmit}
          layout="vertical"
          autoComplete="off"
        >
          <FormItem label="域名" field="domain" rules={[{ required: true }]}>
            <Input placeholder="example.com" />
          </FormItem>

          <FormItem label="证书类型" field="cert_type" initialValue="letsencrypt">
            <Select placeholder="选择证书类型">
              <Option value="letsencrypt">Let's Encrypt (自动)</Option>
              <Option value="custom">自定义证书</Option>
            </Select>
          </FormItem>

          <FormItem
            noStyle
            shouldUpdate={(prev, next) => prev.cert_type !== next.cert_type}
          >
            {(values) => {
              if (values.cert_type === 'custom') {
                return (
                  <>
                    <FormItem label="证书内容 (PEM)" field="cert_pem" rules={[{ required: true }]}>
                      <Input.TextArea placeholder="-----BEGIN CERTIFICATE-----" rows={6} />
                    </FormItem>
                    <FormItem label="私钥 (PEM)" field="key_pem" rules={[{ required: true }]}>
                      <Input.TextArea placeholder="-----BEGIN PRIVATE KEY-----" rows={6} />
                    </FormItem>
                  </>
                )
              }
              return null
            }}
          </FormItem>
        </Form>
      </Modal>
    </div>
  )
}

export default Certificates
