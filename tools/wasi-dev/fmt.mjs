// rustfmt без установленного Rust: npm-пакет @scalar/rust-fmt (rustfmt в wasm).
// Проверено на main: файлы workspace он оставляет как есть, то есть совпадает
// с `cargo fmt --all -- --check` в CI. Без --write только печатает, что не так.
//   node tools/wasi-dev/fmt.mjs <каталог пакета npm> [--write] <каталоги…>
import { readFileSync, writeFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

const [pkg, ...rest] = process.argv.slice(2)
const write = rest.includes('--write')
const roots = rest.filter((arg) => arg !== '--write')
const entry = join(pkg, 'node_modules/@scalar/rust-fmt/dist/index.js')
const { format } = await import(pathToFileURL(entry).href)

const files = []
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    if (name === 'target' || name === '.git' || name === 'node_modules') continue
    const path = join(dir, name)
    if (statSync(path).isDirectory()) walk(path)
    else if (name.endsWith('.rs')) files.push(path)
  }
}
roots.forEach(walk)

let changed = 0
let failed = 0
for (const file of files) {
  const source = readFileSync(file, 'utf8')
  let out
  try {
    out = await format(source, { edition: '2024', styleEdition: '2024' })
  } catch (error) {
    failed++
    console.log(`ошибка ${file}: ${String(error).slice(0, 200)}`)
    continue
  }
  if (out !== source) {
    changed++
    console.log(`${write ? 'исправлен' : 'не отформатирован'} ${file}`)
    if (write) writeFileSync(file, out)
  }
}
console.log(`файлов ${files.length}, расхождений ${changed}, ошибок ${failed}`)
process.exit(failed > 0 || (changed > 0 && !write) ? 1 : 0)
