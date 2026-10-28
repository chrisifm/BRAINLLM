import { useRef, useState } from 'react'
import './Todo.css'

export default function Todo({ title = 'To do' }) {
  const [items, setItems] = useState([])
  const [text, setText] = useState('')
  const [filter, setFilter] = useState('all')
  const nextId = useRef(1)

  const visible = filter === 'done' ? items.filter(it => it.done)
    : filter === 'open' ? items.filter(it => !it.done)
    : items
  const left = items.filter(it => !it.done).length

  const add = (e) => {
    e.preventDefault()
    const t = text.trim()
    if (t === '') return
    setItems([...items, { id: nextId.current++, text: t, done: false }])
    setText('')
  }
  const toggle = (id) => setItems(items.map(it => it.id === id ? { ...it, done: !it.done } : it))
  const remove = (id) => setItems(items.filter(it => it.id !== id))
  const tab = (f, label) => (
    <button className={filter === f ? 'on' : undefined} onClick={() => setFilter(f)}>{label}</button>
  )

  return (
    <section className="todo">
      <h2>{title}</h2>
      <form onSubmit={add}>
        <input value={text} onChange={e => setText(e.target.value)} placeholder="What needs doing?" aria-label="New task" />
        <button disabled={text.trim() === ''}>Add</button>
      </form>
      <ul>
        {visible.length === 0 ? <li className="empty">Nothing here.</li> : visible.map(it => (
          <li key={it.id} className={it.done ? 'done' : undefined}>
            <input type="checkbox" checked={it.done} onChange={() => toggle(it.id)} aria-label={`Mark ${it.text}`} />
            <span>{it.text}</span>
            <button onClick={() => remove(it.id)} aria-label="Delete">×</button>
          </li>
        ))}
      </ul>
      <footer>
        <span>{left} left</span>
        {tab('all', 'All')}{tab('open', 'Open')}{tab('done', 'Done')}
      </footer>
    </section>
  )
}
