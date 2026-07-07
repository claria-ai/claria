import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import Console from './pages/Console.tsx'
import LockGate from './components/LockGate.tsx'

const isConsoleWindow = window.location.hash === '#console'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <LockGate>
      {isConsoleWindow ? <Console /> : <App />}
    </LockGate>
  </StrictMode>,
)
