import { defineConfig } from 'vite';

export default defineConfig({
  base: '/ui/',
  build: {
    outDir: 'dist',
  },
  server: {
    proxy: {
      '/chat': 'http://localhost:8080',
      '/.well-known': 'http://localhost:8080',
      '/health': 'http://localhost:8080',
      '/budget': 'http://localhost:8080',
      '/task': 'http://localhost:8080',
      '/tasks': 'http://localhost:8080',
      '/status': 'http://localhost:8080',
      '/message': 'http://localhost:8080',
      '/agent': 'http://localhost:8080',
      '/conversations': 'http://localhost:8080',
      '/schedules': 'http://localhost:8080',
      '/output': 'http://localhost:8080',
      '/files': 'http://localhost:8080',
      '/decrypt': 'http://localhost:8080',
      '/rates': 'http://localhost:8080',
      '/certificates': 'http://localhost:8080',
      '/compliance': 'http://localhost:8080',
      '/lifecycle': 'http://localhost:8080',
      '/staged': 'http://localhost:8080',
      '/audit': 'http://localhost:8080',
      '/v1': 'http://localhost:8080',
      '/heartbeat': 'http://localhost:8080',
      '/memory': 'http://localhost:8080',
      '/services': 'http://localhost:8080',
      '/wallet': 'http://localhost:8080',
      '/artifacts': 'http://localhost:8080',
    },
  },
});
