// A stopped or malformed probe cannot prove that a worker warning is present or absent.
export function hasDoctorAdvice(result, code) {
  if (result.error || result.signal || ![0, 1].includes(result.status)) {
    throw Error(`Doctor queue probe did not complete successfully: status=${result.status}, error=${result.error?.code ?? 'none'}, signal=${result.signal ?? 'none'}`)
  }
  let report
  try { report = JSON.parse(result.stdout) } catch { throw Error('Doctor queue probe returned invalid JSON') }
  const adviceValid = Array.isArray(report?.advice)
    && report.advice.every(item => item && typeof item === 'object' && typeof item.code === 'string')
  if (result.status !== 0 || report?.ok !== true || !adviceValid) {
    // Report only the compiler's finite gate labels, never arbitrary messages or source values.
    const gates = new Set(['check', 'tests', 'leaks', 'build', 'seo', 'env', 'seoBar', 'staticArtifact'])
    const failed = Array.isArray(report?.failed) ? report.failed.filter(name => gates.has(name)) : []
    throw Error(`Doctor queue probe returned an unhealthy or malformed report: status=${result.status}, failed=${failed.join(',') || 'unknown'}, adviceValid=${adviceValid}`)
  }
  return report.advice.some(item => item.code === code)
}
