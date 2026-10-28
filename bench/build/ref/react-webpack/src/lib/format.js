// Shared formatting helpers (port of examples/lib/format.jsbrain).
const money = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' })
const day = new Intl.DateTimeFormat('en-US', { dateStyle: 'medium' })

export default {
  money: (n) => money.format(n),
  date: (iso) => day.format(new Date(iso)),
  initials: (name) => name.split(' ').filter(p => p !== '').slice(0, 2).map(p => p[0].toUpperCase()).join(''),
  plural: (n, one, many) => `${n} ${n === 1 ? one : many}`,
}
