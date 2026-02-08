import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import axios from 'axios'

export const useAuthStore = create(
  persist(
    (set, get) => ({
      token: null,
      user: null,

      setToken: (token) => {
        set({ token })
        if (token) {
          axios.defaults.headers.common['Authorization'] = `Bearer ${token}`
        } else {
          delete axios.defaults.headers.common['Authorization']
        }
      },

      setUser: (user) => set({ user }),

      login: async (credentials) => {
        try {
          const response = await axios.post('/api/auth/login', credentials)
          if (response.data.code === 200) {
            const { token, user } = response.data.data
            get().setToken(token)
            set({ user })
            return { success: true }
          }
          return { success: false, message: response.data.message }
        } catch (error) {
          return {
            success: false,
            message: error.response?.data?.message || '登录失败'
          }
        }
      },

      logout: async () => {
        try {
          await axios.post('/api/auth/logout')
        } finally {
          get().setToken(null)
          set({ user: null })
        }
      },

      initAuth: () => {
        const token = get().token
        if (token) {
          axios.defaults.headers.common['Authorization'] = `Bearer ${token}`
        }
      }
    }),
    {
      name: 'auth-storage',
      partialize: (state) => ({ token: state.token, user: state.user })
    }
  )
)
