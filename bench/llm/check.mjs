// A failed compiler process must never become an empty successful diagnostic list.
export function checkedDiagnostics(raw, status, rel) {
  if (![0,1].includes(status) || typeof raw !== 'string' || !raw.trim()) throw Error('Compiler infrastructure failed before producing diagnostics')
  let all
  try { all=JSON.parse(raw) } catch { throw Error('Compiler returned invalid diagnostic JSON') }
  if (!Array.isArray(all) || all.some(d=>!d || typeof d.code!=='string' || typeof d.msg!=='string' || d.file!==undefined&&typeof d.file!=='string')) throw Error('Compiler returned an invalid diagnostic shape')
  const errors=all.filter(d=>(d.level??'error')==='error')
  if (status===1&&!errors.length || status===0&&errors.length) throw Error('Compiler exit status disagrees with its diagnostics')
  const base=rel.split('/').pop()
  const own=errors.filter(d=>!d.file||d.file.endsWith('/'+base)||d.file===base||d.file.endsWith(rel))
  if (errors.length&&!own.length) throw Error('Compiler rejected scaffold files outside the generated source')
  return own
}
