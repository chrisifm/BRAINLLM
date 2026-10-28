import { mount } from 'svelte'
import './tokens.css'
import './app.css'
import Dashboard from './views/Dashboard.svelte'

mount(Dashboard, { target: document.getElementById('content') })
