import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import Console from './pages/Console.tsx'
import ErrorBoundary from './components/ErrorBoundary.tsx'
import LockGate from './components/LockGate.tsx'
import { installGlobalErrorHandlers } from './lib/logBridge.ts'

installGlobalErrorHandlers()

const isConsoleWindow = window.location.hash === '#console'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ErrorBoundary>
      <LockGate>
        {isConsoleWindow ? <Console /> : <App />}
      </LockGate>
    </ErrorBoundary>
  </StrictMode>,
)
