import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import axios from 'axios'

export const useUserStore = defineStore('user', () => {
  // State
  const token = ref(localStorage.getItem('token') || '')
  const userInfo = ref(null)

  // Getters
  const isLoggedIn = computed(() => !!token.value)

  // Actions
  const setToken = (newToken) => {
    token.value = newToken
    localStorage.setItem('token', newToken)
    axios.defaults.headers.common['Authorization'] = `Bearer ${newToken}`
  }

  const clearToken = () => {
    token.value = ''
    userInfo.value = null
    localStorage.removeItem('token')
    delete axios.defaults.headers.common['Authorization']
  }

  const login = async (credentials) => {
    try {
      const response = await axios.post('/api/auth/login', credentials)
      if (response.data.code === 200) {
        setToken(response.data.data.token)
        userInfo.value = response.data.data.user
        return { success: true }
      }
      return { success: false, message: response.data.message }
    } catch (error) {
      return { 
        success: false, 
        message: error.response?.data?.message || '登录失败'
      }
    }
  }

  const logout = async () => {
    try {
      await axios.post('/api/auth/logout')
    } finally {
      clearToken()
    }
  }

  // 初始化 axios 头部
  if (token.value) {
    axios.defaults.headers.common['Authorization'] = `Bearer ${token.value}`
  }

  return {
    token,
    userInfo,
    isLoggedIn,
    setToken,
    clearToken,
    login,
    logout
  }
})
