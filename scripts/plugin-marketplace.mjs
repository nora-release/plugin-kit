#!/usr/bin/env node
import { readFileSync, realpathSync } from 'node:fs'
import { registerHooks, stripTypeScriptTypes } from 'node:module'
import { fileURLToPath } from 'node:url'

const sourceURL = new URL('./plugin-marketplace.ts', import.meta.url)
registerHooks({
  load(url, context, nextLoad) {
    if (url !== sourceURL.href) return nextLoad(url, context)
    return { format: 'module', shortCircuit: true,
      source: stripTypeScriptTypes(readFileSync(sourceURL, 'utf8'), { mode: 'strip', sourceUrl: url }) }
  },
})

export const { validateManifest, repositoryPath, validateEntry, validateRegistry, formatRegistry,
  manifestFromArchive, verifyArchive, downloadRelease, verifyDownloads, runMarketplaceCLI } = await import(sourceURL.href)

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) runMarketplaceCLI()
