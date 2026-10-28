import { useEffect, useState } from 'react'
import './Counter.css'

export default function Counter({ start = 0, label = 'Clicks' }) {
  const [n, setN] = useState(start)
  const dbl = n * 2
  useEffect(() => { if (import.meta.env.DEV) console.log(n) }, [n])
  return (
    <button className={'counter' + (n > 10 ? ' hot' : '')} onClick={() => setN(n + 1)}>
      {label}: {n} × 2 = {dbl}
    </button>
  )
}
