import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    // 开发时将API请求代理到本地后端，避免跨域问题
    proxy: {
      '/api': 'http://localhost:8080',
      '/gw': 'http://localhost:8080',
      '/sandbox': 'http://localhost:8080',
    }
  }
})
