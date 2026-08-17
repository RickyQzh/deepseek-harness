/** Named Vitest smoke: spawn `target/debug/dsh web` when `DSH_RUNTIME=rust`. */

import { spawn } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'

const rust = process.env.DSH_RUNTIME === 'rust'

describe.skipIf(!rust)('rust dsh web smoke', () => {
  const bin = process.env.DSH_RUNTIME_BIN && process.env.DSH_RUNTIME_BIN !== ''
    ? process.env.DSH_RUNTIME_BIN
    : join(process.cwd(), 'target/debug/dsh')
  const dist = mkdtempSync(join(tmpdir(), 'dsh-web-dist-'))
  const root = mkdtempSync(join(tmpdir(), 'dsh-web-root-'))
  const clients = mkdtempSync(join(tmpdir(), 'dsh-web-clients-'))
  writeFileSync(join(dist, 'index.html'), '<html><head></head><body>ok</body></html>')
  mkdirSync(join(root, 'sessions'), { recursive: true })
  mkdirSync(join(root, 'home'), { recursive: true })
  let baseUrl = ''
  let child: ReturnType<typeof spawn> | undefined

  afterAll(() => {
    child?.kill('SIGTERM')
    rmSync(dist, { recursive: true, force: true })
    rmSync(root, { recursive: true, force: true })
    rmSync(clients, { recursive: true, force: true })
  })

  it('prints the ready URL, serves index, describes the host, and 426s mux GET', async () => {
    if (!existsSync(bin)) {
      throw new Error(`DSH_RUNTIME=rust but ${bin} is missing; run cargo build -p dsh-cli`)
    }
    const env = { ...process.env }
    delete env.DSH_CORDIS_CONFIG
    env.DSH_SESSION_ROOT = join(root, 'sessions')
    env.DSH_CWD = root
    env.DSH_HOME = join(root, 'home')
    env.DSH_CLIENT_PACKAGES = clients
    child = spawn(bin, ['web', '--port', '0', '--dist', dist], {
      env,
      cwd: process.cwd(),
    })
    let out = ''
    baseUrl = await new Promise<string>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`not ready: ${out}`)), 90_000)
      const onData = (chunk: Buffer): void => {
        out += chunk.toString()
        const match = /dsh web: (http:\/\/[^\s]+)/.exec(out)
        if (match?.[1] !== undefined) {
          clearTimeout(timer)
          resolve(match[1])
        }
      }
      child?.stdout?.on('data', onData)
      child?.stderr?.on('data', onData)
      child?.once('exit', (code) => {
        clearTimeout(timer)
        reject(new Error(`exited ${String(code)}: ${out}`))
      })
    })
    const index = await fetch(baseUrl)
    expect(index.status).toBe(200)
    const html = await index.text()
    expect(html.includes('window.__DSH_BOOT__') || html.includes('<html')).toBe(true)
    const described = await fetch(`${baseUrl}/api/host.describe`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ type: 'client-request', rpcId: 'r1', method: 'host.describe', payload: {} }),
    })
    expect(described.status).toBe(200)
    const body = await described.json() as { result: { ok: boolean; value?: { canOpenPath?: boolean } } }
    expect(body.result.ok).toBe(true)
    expect(body.result.value?.canOpenPath).toBe(false)
    const mux = await fetch(`${baseUrl}/api/events.mux`)
    expect(mux.status).toBe(426)
  }, 120_000)
})
