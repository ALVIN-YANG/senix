<template>
  <div class="dashboard">
    <h1>系统概览</h1>
    
    <!-- 统计卡片 -->
    <el-row :gutter="20" class="stats-row">
      <el-col :span="6">
        <el-card class="stat-card" :body-style="{ padding: '20px' }">
          <div class="stat-item">
            <div class="stat-icon sites">
              <el-icon size="40"><OfficeBuilding /></el-icon>
            </div>
            <div class="stat-info">
              <div class="stat-value">{{ siteStats.total || 0 }}</div>
              <div class="stat-label">站点总数</div>
            </div>
          </div>
        </el-card>
      </el-col>
      
      <el-col :span="6">
        <el-card class="stat-card" :body-style="{ padding: '20px' }">
          <div class="stat-item">
            <div class="stat-icon active">
              <el-icon size="40"><CircleCheck /></el-icon>
            </div>
            <div class="stat-info">
              <div class="stat-value">{{ siteStats.active || 0 }}</div>
              <div class="stat-label">运行中</div>
            </div>
          </div>
        </el-card>
      </el-col>
      
      <el-col :span="6">
        <el-card class="stat-card" :body-style="{ padding: '20px' }">
          <div class="stat-item">
            <div class="stat-icon certs">
              <el-icon size="40"><Lock /></el-icon>
            </div>
            <div class="stat-info">
              <div class="stat-value">{{ certStats.total || 0 }}</div>
              <div class="stat-label">证书总数</div>
            </div>
          </div>
        </el-card>
      </el-col>
      
      <el-col :span="6">
        <el-card class="stat-card" :body-style="{ padding: '20px' }">
          <div class="stat-item">
            <div class="stat-icon warning">
              <el-icon size="40"><Warning /></el-icon>
            </div>
            <div class="stat-info">
              <div class="stat-value">{{ certStats.expiring_soon || 0 }}</div>
              <div class="stat-label">即将过期</div>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>
    
    <!-- 工作模式分布 -->
    <el-row :gutter="20" class="charts-row">
      <el-col :span="12">
        <el-card>
          <template #header>
            <span>工作模式分布</span>
          </template>
          <div class="mode-stats">
            <div class="mode-item">
              <span class="mode-label">独立代理模式</span>
              <el-progress :percentage="getModePercentage('standalone')" />
            </div>
            <div class="mode-item">
              <span class="mode-label">证书管理模式</span>
              <el-progress :percentage="getModePercentage('cert_only')" color="#67C23A" />
            </div>
            <div class="mode-item">
              <span class="mode-label">配置生成模式</span>
              <el-progress :percentage="getModePercentage('config_only')" color="#E6A23C" />
            </div>
          </div>
        </el-card>
      </el-col>
      
      <el-col :span="12">
        <el-card>
          <template #header>
            <span>SSL 证书状态</span>
          </template>
          <div class="cert-stats">
            <div class="cert-stat-item">
              <div class="cert-value">{{ certStats.active || 0 }}</div>
              <div class="cert-label">有效证书</div>
            </div>
            <div class="cert-stat-item">
              <div class="cert-value warning">{{ certStats.expiring_soon || 0 }}</div>
              <div class="cert-label">即将过期</div>
            </div>
            <div class="cert-stat-item">
              <div class="cert-value danger">{{ certStats.expired || 0 }}</div>
              <div class="cert-label">已过期</div>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>
    
    <!-- 快捷操作 -->
    <el-card class="quick-actions">
      <template #header>
        <span>快捷操作</span>
      </template>
      <div class="actions">
        <el-button type="primary" @click="$router.push('/sites')">
          <el-icon><Plus /></el-icon>
          添加站点
        </el-button>
        <el-button type="success" @click="$router.push('/certs')">
          <el-icon><Lock /></el-icon>
          申请证书
        </el-button>
        <el-button @click="generateAllConfigs">
          <el-icon><Refresh /></el-icon>
          生成所有配置
        </el-button>
      </div>
    </el-card>
  </div>
</template>

<script setup>
import { ref, onMounted, computed } from 'vue'
import axios from 'axios'
import { ElMessage } from 'element-plus'

const siteStats = ref({})
const certStats = ref({})

const getModePercentage = (mode) => {
  const total = siteStats.value.total || 0
  if (total === 0) return 0
  
  const count = siteStats.value[mode] || 0
  return Math.round((count / total) * 100)
}

const fetchStats = async () => {
  try {
    const [siteRes, certRes] = await Promise.all([
      axios.get('/api/sites/stats'),
      axios.get('/api/certs/stats')
    ])
    
    if (siteRes.data.code === 200) {
      siteStats.value = siteRes.data.data
    }
    if (certRes.data.code === 200) {
      certStats.value = certRes.data.data
    }
  } catch (error) {
    console.error('Failed to fetch stats:', error)
  }
}

const generateAllConfigs = async () => {
  try {
    const res = await axios.post('/api/sites/generate-all')
    if (res.data.code === 200) {
      ElMessage.success('配置生成成功')
    } else {
      ElMessage.error(res.data.message)
    }
  } catch (error) {
    ElMessage.error('配置生成失败')
  }
}

onMounted(() => {
  fetchStats()
})
</script>

<style scoped>
.dashboard h1 {
  margin-bottom: 20px;
  color: #303133;
}

.stats-row {
  margin-bottom: 20px;
}

.stat-card {
  transition: all 0.3s;
}

.stat-card:hover {
  transform: translateY(-5px);
  box-shadow: 0 2px 12px 0 rgba(0,0,0,0.1);
}

.stat-item {
  display: flex;
  align-items: center;
}

.stat-icon {
  width: 60px;
  height: 60px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-right: 15px;
}

.stat-icon.sites {
  background-color: #ecf5ff;
  color: #409EFF;
}

.stat-icon.active {
  background-color: #f0f9eb;
  color: #67C23A;
}

.stat-icon.certs {
  background-color: #f5f7fa;
  color: #909399;
}

.stat-icon.warning {
  background-color: #fdf6ec;
  color: #E6A23C;
}

.stat-value {
  font-size: 24px;
  font-weight: bold;
  color: #303133;
}

.stat-label {
  font-size: 14px;
  color: #909399;
  margin-top: 5px;
}

.charts-row {
  margin-bottom: 20px;
}

.mode-stats {
  padding: 10px 0;
}

.mode-item {
  margin-bottom: 20px;
}

.mode-item:last-child {
  margin-bottom: 0;
}

.mode-label {
  display: block;
  margin-bottom: 8px;
  color: #606266;
}

.cert-stats {
  display: flex;
  justify-content: space-around;
  padding: 20px 0;
}

.cert-stat-item {
  text-align: center;
}

.cert-value {
  font-size: 32px;
  font-weight: bold;
  color: #67C23A;
}

.cert-value.warning {
  color: #E6A23C;
}

.cert-value.danger {
  color: #F56C6C;
}

.cert-label {
  margin-top: 8px;
  color: #909399;
}

.quick-actions {
  margin-top: 20px;
}

.actions {
  display: flex;
  gap: 10px;
}

.actions .el-button {
  display: flex;
  align-items: center;
  gap: 5px;
}
</style>
