import { createRoot } from 'react-dom/client'
import './tokens.css'
import './app.css'
import Dashboard from './views/Dashboard.jsx'

createRoot(document.getElementById('content')).render(<Dashboard />)
