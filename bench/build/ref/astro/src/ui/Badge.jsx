import './Badge.css'

export default function Badge({ tone = 'neutral', text }) {
  return <span className="badge" data-tone={tone}>{text}</span>
}
