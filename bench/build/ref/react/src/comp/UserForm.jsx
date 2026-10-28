import { useState } from 'react'
import './UserForm.css'

export default function UserForm({ user = null, busy = false, onSave, onCancel }) {
  const [name, setName] = useState(user ? user.name : '')
  const [email, setEmail] = useState(user ? user.email : '')
  const [active, setActive] = useState(user ? user.active : true)

  const valid = name.trim() !== '' && email.includes('@')

  const save = (e) => {
    e.preventDefault()
    if (!valid || busy) return
    onSave?.({ id: user ? user.id : null, name: name.trim(), email: email.trim(), active })
  }

  return (
    <form className="user-form" onSubmit={save}>
      <label>Name <input value={name} onChange={e => setName(e.target.value)} /></label>
      <label>Email <input type="email" value={email} onChange={e => setEmail(e.target.value)} /></label>
      <label><input type="checkbox" checked={active} onChange={e => setActive(e.target.checked)} /> Active</label>
      <button disabled={!valid || busy}>{busy ? 'Saving…' : 'Save'}</button>
      <button type="button" onClick={() => onCancel?.()}>Cancel</button>
    </form>
  )
}
