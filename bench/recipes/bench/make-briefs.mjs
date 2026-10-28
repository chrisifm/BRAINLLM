// Builds the two brief sets of the recipes benchmark (task #83) from briefs-src.json:
//   briefs-83.json     the plain brief (recipes off)
//   briefs-83-on.json  the same brief + the mapped recipe inlined as context (recipes on)
// The autonomy harness cannot pre-seed files into the app, so the "on" variant hands the recipe's files to the agent
// in the brief and asks it to write them first; their tool calls and tokens count against the "on" variant.
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const HERE = new URL('.', import.meta.url).pathname
const RECIPES = join(HERE, '..')
const src = JSON.parse(readFileSync(join(HERE, 'briefs-src.json'), 'utf8'))

const off = src.map(({ id, brief, pages }) => ({ id, brief, pages }))
const on = src.map(({ id, brief, pages, recipe }) => {
  const m = JSON.parse(readFileSync(join(RECIPES, recipe, 'recipe.json'), 'utf8'))
  const files = m.files.map((f) => `=== ${f} ===\n${readFileSync(join(RECIPES, recipe, f), 'utf8').trimEnd()}`).join('\n\n')
  const ctx = `Start from the ${recipe} recipe in bench/recipes/${recipe} (${m.title}). It already passes brain check, brain doctor and brain test in a fresh brain_new app. Its files are below, each with its path relative to the app root: after brain_new, write them as they are (they replace the scaffold files with the same path), then adapt names, routes and texts to the brief, keeping the structure and the tests passing.\nRecipe notes: ${m.notes}\n\n${files}`
  return { id, brief: `${brief}\n\n${ctx}`, pages }
})

writeFileSync(join(HERE, 'briefs-83.json'), JSON.stringify(off, null, 2) + '\n')
writeFileSync(join(HERE, 'briefs-83-on.json'), JSON.stringify(on, null, 2) + '\n')
console.log(on.map((b) => `${b.id}: ${b.brief.length} chars`).join('\n'))
