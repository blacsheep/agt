// The architecture docs are Chinese only. These English pages keep the
// language switcher working and point readers to the Chinese originals.
import { readdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const dir = fileURLToPath(new URL('../zh/architecture/', import.meta.url))

export default {
  paths() {
    return readdirSync(dir)
      .filter((file) => file.endsWith('.md') && file !== 'index.md')
      .map((file) => {
        const page = file.slice(0, -3)
        const title = readFileSync(dir + file, 'utf8').match(/^# (.+)$/m)?.[1] ?? page
        return { params: { page, title } }
      })
  },
}
