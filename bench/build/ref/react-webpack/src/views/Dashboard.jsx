import { useEffect, useState } from 'react'
import Badge from '../ui/Badge.jsx'
import UserForm from '../comp/UserForm.jsx'
import api from '../lib/api.js'
import fmt from '../lib/format.js'
import './Dashboard.css'

export default function Dashboard() {
  const [users, setUsers] = useState([])
  const [query, setQuery] = useState('')
  const [editing, setEditing] = useState(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Equivalent of `load()`: there is no server in a static Vite build, so it runs on mount.
  useEffect(() => { api.list().then(setUsers) }, [])

  const q = query.trim().toLowerCase()
  const shown = q === '' ? users
    : users.filter(u => u.name.toLowerCase().includes(q) || u.email.toLowerCase().includes(q))
  const activeCount = users.filter(u => u.active).length

  const save = async (data) => {
    setBusy(true)
    setError('')
    try {
      const saved = data.id === null ? await api.create(data) : await api.update(data)
      setUsers(prev => data.id === null ? [...prev, saved] : prev.map(u => u.id === saved.id ? saved : u))
      setEditing(null)
    } catch (e) {
      setError(e.message)
    }
    setBusy(false)
  }

  const remove = async (id) => {
    setError('')
    try {
      await api.remove(id)
      setUsers(prev => prev.filter(u => u.id !== id))
    } catch (e) {
      setError(e.message)
    }
  }

  return (
    <main className="dash">
      <header>
        <h1>Users</h1>
        <span>{activeCount} active of {fmt.plural(users.length, 'user', 'users')}</span>
        <input value={query} onChange={e => setQuery(e.target.value)} placeholder="Search by name or email" aria-label="Search users" />
        <button onClick={() => setEditing({})}>New</button>
      </header>

      {error !== '' && <p className="error" role="alert">{error}</p>}

      {editing !== null && (
        <UserForm key={editing.id ?? 'new'} user={editing.id ? editing : null} busy={busy} onSave={save} onCancel={() => setEditing(null)} />
      )}

      <table>
        <thead><tr><th>Name</th><th>Email</th><th>Status</th><th></th></tr></thead>
        <tbody>
          {shown.length === 0 ? (
            <tr><td colSpan={4}>{query === '' ? 'No users yet.' : 'No results for this search.'}</td></tr>
          ) : shown.map(u => (
            <tr key={u.id}>
              <td>{u.name}</td>
              <td>{u.email}</td>
              <td><Badge tone={u.active ? 'ok' : 'bad'} text={u.active ? 'Active' : 'Inactive'} /></td>
              <td>
                <button onClick={() => setEditing(u)}>Edit</button>
                <button onClick={() => remove(u.id)}>Delete</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </main>
  )
}
