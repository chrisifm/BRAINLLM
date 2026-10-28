// Fake in-memory API for the dashboard example (plain JS escape hatch, not compiled).
let db = [
  { id: 1, name: 'Ada Lovelace', email: 'ada@example.com', active: true },
  { id: 2, name: 'Alan Turing', email: 'alan@example.com', active: false },
]
let seq = 3
const wait = (v) => new Promise((r) => setTimeout(() => r(v), 50))

export default {
  list: () => wait(db.map((u) => ({ ...u }))),
  create: (u) => { const n = { ...u, id: seq++ }; db.push(n); return wait({ ...n }) },
  update: (u) => { db = db.map((x) => (x.id === u.id ? { ...u } : x)); return wait({ ...u }) },
  remove: (id) => { db = db.filter((x) => x.id !== id); return wait(true) },
}
