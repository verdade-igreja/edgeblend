// Fails the build if the phone bundle could reach the internet or render server strings as
// HTML. The phone may have no internet, and the page must never trust server text as markup.
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

const errors = []
const html = readFileSync('dist/index.html', 'utf8')
// Any http(s) URL in a src/href/url()/@import is an external fetch.
const external = html.match(/(?:src|href)\s*=\s*["']https?:\/\/[^"']+|url\(\s*["']?https?:\/\/[^)"']+|@import\s*(?:url\()?["']?https?:\/\/[^)"';]+/gi)
if (external) errors.push(`external resources in dist/index.html: ${[...new Set(external)].join(', ')}`)
if (/<script[^>]+src=|<link[^>]+rel=["']?stylesheet/i.test(html)) errors.push('dist/index.html is not self-contained')

function walk(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) walk(p)
    else if (/\.(vue|ts)$/.test(name)) {
      const src = readFileSync(p, 'utf8')
      if (/v-html|innerHTML/.test(src)) errors.push(`${p}: v-html/innerHTML is not allowed`)
    }
  }
}
walk('src')

if (errors.length) {
  console.error(errors.join('\n'))
  process.exit(1)
}
console.log(`bundle ok (${(html.length / 1024).toFixed(0)} kB)`)
