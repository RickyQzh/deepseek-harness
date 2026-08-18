// Shared plumbing for the web smoke tests (dist location, free port, failure shots).
import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync, mkdirSync } from 'node:fs'
import { mkdtemp, readdir, readFile, symlink } from 'node:fs/promises'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { Browser, Page } from 'playwright'

/** The built page under test; `pnpm run test:web` rebuilds it before running. */
export const DIST_INDEX = fileURLToPath(new URL('../dist/index.html', import.meta.url))

export const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url))

/**
 * Browser language a page must advertise to boot into the product's Chinese
 * surface: with no stored preference the client derives its initial locale
 * from the browser, and Playwright's default browser asks for English.
 */
export const ZH_BROWSER_LOCALE = 'zh-CN'

/**
 * Open the standard browser-test page advertising English before client boot.
 * This keeps role locators and goldens deterministic while leaving the Host
 * settings document free to override the provisional browser-derived locale;
 * scenarios asserting the Chinese surface advertise
 * {@link ZH_BROWSER_LOCALE} instead.
 * @param browser - Playwright browser owning the page.
 * @param height - Viewport height; width is fixed to the lane baseline.
 * @returns the initialized page.
 */
export async function newEnglishPage(browser: Browser, height = 1000): Promise<Page> {
  return await browser.newPage({ viewport: { width: 1680, height }, locale: 'en-US' })
}

/** Fail loud on a stale checkout instead of testing yesterday's bundle. */
export function requireDist(): void {
  if (!existsSync(DIST_INDEX)) {
    throw new Error('web app dist not built — run `pnpm run build` from the repository root (`pnpm run test:web` does this first)')
  }
}

/** Built SPA directory passed as `dsh web --dist`. */
export const DIST_DIR = dirname(DIST_INDEX)

/**
 * Packages a rust one-level scan would otherwise activate together, unlike
 * the Node loader roster: HMR is disabled in the web-app patch, and native vs
 * browse directory pickers are exclusive (rust host.describe reports
 * canOpenPath false, so browse is the occupant).
 */
const SKIP_WEB_CLIENT_PACKAGES = new Set([
  '@deepseek-ai/dsh-client-hmr',
  '@deepseek-ai/dsh-client-ui-directory-picker-native',
])

/** True when this Vitest process should spawn the Rust `dsh` web bin. */
export function rustWebRuntime(): boolean {
  return process.env.DSH_RUNTIME === 'rust'
}

/**
 * Resolve the Rust `dsh` bin and fail if it is missing.
 * @returns `DSH_RUNTIME_BIN` when set and non-empty, else `target/debug/dsh`.
 */
export function requireRustDshBin(): string {
  const bin = process.env.DSH_RUNTIME_BIN !== undefined && process.env.DSH_RUNTIME_BIN !== ''
    ? process.env.DSH_RUNTIME_BIN
    : join(REPO_ROOT, 'target/debug/dsh')
  if (!existsSync(bin)) {
    throw new Error(`DSH_RUNTIME=rust but ${bin} is missing; run cargo build -p dsh-cli`)
  }
  return bin
}

/**
 * Temp directory of one symlink per built web `dsh.client` package so rust
 * one-level package.json scan sees typert, api, client, and extension bundles.
 * Omits `@deepseek-ai/dsh-client-hmr` and
 * `@deepseek-ai/dsh-client-ui-directory-picker-native`.
 * HTTP-only smoke keeps an empty temp dir instead of this roster.
 * @returns the staging directory; the caller removes it.
 */
export async function stageWebClientPackages(): Promise<string> {
  const staged = await mkdtemp(join(tmpdir(), 'dsh-web-clients-'))
  const packagesRoot = join(REPO_ROOT, 'packages')
  const missing: string[] = []
  const groups = await readdir(packagesRoot, { withFileTypes: true })
  for (const group of groups) {
    if (!group.isDirectory()) continue
    const groupDir = join(packagesRoot, group.name)
    const pkgs = await readdir(groupDir, { withFileTypes: true })
    for (const pkg of pkgs) {
      if (!pkg.isDirectory()) continue
      const dir = join(groupDir, pkg.name)
      const manifestPath = join(dir, 'package.json')
      if (!existsSync(manifestPath)) continue
      const data = JSON.parse(await readFile(manifestPath, 'utf8')) as {
        name?: unknown
        dsh?: { client?: { platform?: unknown } }
      }
      if (data.dsh?.client?.platform !== 'web') continue
      const name = typeof data.name === 'string' ? data.name : pkg.name
      if (SKIP_WEB_CLIENT_PACKAGES.has(name)) continue
      const bundle = join(dir, 'lib', 'client.js')
      if (!existsSync(bundle)) {
        missing.push(`${name} ${bundle}`)
        continue
      }
      const link = join(staged, pkg.name)
      if (existsSync(link)) {
        throw new Error(`duplicate web client package folder name ${pkg.name}`)
      }
      await symlink(dir, link)
    }
  }
  if (missing.length > 0) {
    throw new Error(
      `client bundle not found; run pnpm run build before launch: ${missing.join('; ')}`,
    )
  }
  const stagedPkgs = await readdir(staged)
  if (stagedPkgs.length === 0) {
    throw new Error('no web dsh.client packages found under packages/')
  }
  return staged
}

/**
 * Child env for `dsh web`: isolated persist/cwd/home, empty or explicit client
 * packages, and no inherited `DSH_CORDIS_CONFIG` (unset uses bundled `WEB_YAML`).
 * @param input - persist, cwd, home, and client-package directory.
 * @returns a copy of `process.env` with those keys applied.
 */
export function rustWebChildEnv(input: {
  sessionRoot: string
  cwd: string
  home: string
  clientPackages: string
}): NodeJS.ProcessEnv {
  const env = { ...process.env }
  delete env.DSH_CORDIS_CONFIG
  env.DSH_SESSION_ROOT = input.sessionRoot
  env.DSH_CWD = input.cwd
  env.DSH_HOME = input.home
  env.DSH_CLIENT_PACKAGES = input.clientPackages
  return env
}

/**
 * Wait for `dsh web: http://…` on stdout or stderr.
 * @param child - spawned `dsh` process.
 * @param timeoutMs - ready timeout; default 90s.
 * @returns the printed origin.
 */
export function waitForDshWebUrl(child: ChildProcess, timeoutMs = 90_000): Promise<string> {
  return new Promise((resolve, reject) => {
    let out = ''
    const timer = setTimeout(() => reject(new Error(`not ready: ${out}`)), timeoutMs)
    const onData = (chunk: Buffer): void => {
      out += chunk.toString()
      const match = /dsh web: (http:\/\/[^\s]+)/.exec(out)
      if (match?.[1] !== undefined) {
        clearTimeout(timer)
        resolve(match[1])
      }
    }
    child.stdout?.on('data', onData)
    child.stderr?.on('data', onData)
    child.once('exit', (code) => {
      clearTimeout(timer)
      reject(new Error(`exited ${String(code)}: ${out}`))
    })
  })
}

/**
 * Spawn `dsh web --port 0 --dist <dist>` with the isolated child env.
 * @param input - dist, persist, cwd, home, and client-package directory.
 * @returns the child and the ready origin.
 */
export async function spawnRustDshWeb(input: {
  dist: string
  sessionRoot: string
  cwd: string
  home: string
  clientPackages: string
}): Promise<{ child: ChildProcess; baseUrl: string }> {
  const child = spawn(requireRustDshBin(), ['web', '--port', '0', '--dist', input.dist], {
    env: rustWebChildEnv(input),
    cwd: REPO_ROOT,
  })
  const baseUrl = await waitForDshWebUrl(child)
  return { child, baseUrl }
}

/**
 * SIGTERM the child, then SIGKILL if it has not exited within 5s.
 * @param child - spawned `dsh` process, if any.
 */
export async function stopRustChild(child: ChildProcess | undefined): Promise<void> {
  if (child === undefined) return
  await new Promise<void>((resolve) => {
    if (child.exitCode !== null) {
      resolve()
      return
    }
    const timer = setTimeout(() => {
      child.kill('SIGKILL')
      resolve()
    }, 5_000)
    child.once('exit', () => {
      clearTimeout(timer)
      resolve()
    })
    child.kill('SIGTERM')
  })
}

/** OS-assigned free port, released before use (the spawned `dsh web` needs a concrete --port). */
export function probeFreePort(): Promise<number> {
  return new Promise((resolvePort, reject) => {
    const probe = createServer()
    probe.once('error', reject)
    probe.listen(0, '127.0.0.1', () => {
      const address = probe.address()
      if (address === null || typeof address === 'string') {
        probe.close(() => { reject(new Error('port probe returned no address')) })
        return
      }
      probe.close(() => { resolvePort(address.port) })
    })
  })
}

/**
 * Drive the hero's workspace picker through the composed directory dialog
 * until the live composer unlocks. A fresh world has no Workspace, so the boot
 * lands in the Workspace-trigger view state (startup auto-selection has nothing to
 * select); every scenario that types into the composer must connect one
 * first. With nothing to list, activating the textarea raises the dialog directly —
 * adding a workspace is the picker's only entry. The directory is staged here
 * and adopted through the path editor, which is idempotent across the repeated
 * connects a scenario may make; creating a folder from inside the dialog (the
 * product's other half of the same route) is covered by
 * workspace-management.e2e.ts. The default name 'workspace' keeps the session
 * header cwd at <root>/workspace, the materialization proof several scenarios
 * assert.
 * @param page - the page under test.
 * @param root - host directory the workspace folder is staged in (the scaffold's `workspaceCwd`).
 * @param name - folder name staged and adopted as the workspace.
 */
export async function connectFreshWorkspace(page: Page, root: string, name = 'workspace'): Promise<void> {
  mkdirSync(join(root, name), { recursive: true })
  await page.getByRole('textbox', { name: 'Choose workspace' }).click()
  const dialog = page.getByRole('dialog', { name: 'Select Workspace Directory' })
  await dialog.waitFor({ timeout: 10_000 })
  await dialog.getByRole('button', { name: 'Edit path' }).click()
  const pathInput = dialog.getByRole('textbox', { name: 'Edit path' })
  await pathInput.fill(join(root, name))
  await pathInput.press('Enter')
  await dialog.getByRole('button', { name: 'Open', exact: true }).click()
  // The pick connected the workspace: the blank session's live composer
  // replaces the locked placeholder and enables.
  await page.locator('textarea:enabled[placeholder="Describe what you want to build"]')
    .waitFor({ timeout: 15_000 })
}

/**
 * {@link connectFreshWorkspace} over the product default Chinese locale: the
 * English helper's anchors assume the locale every other scenario boots, so a
 * scenario that deliberately keeps zh needs the localized picker copy.
 * @param page - the browser page under test.
 * @param root - workspace parent directory.
 * @param name - directory created under `root` and connected.
 */
export async function connectFreshWorkspaceZh(page: Page, root: string, name = 'workspace'): Promise<void> {
  mkdirSync(join(root, name), { recursive: true })
  await page.getByRole('textbox', { name: '选择工作区' }).click()
  const dialog = page.getByRole('dialog', { name: '选择工作区目录' })
  await dialog.waitFor({ timeout: 10_000 })
  await dialog.getByRole('button', { name: '编辑路径' }).click()
  const pathInput = dialog.getByRole('textbox', { name: '编辑路径' })
  await pathInput.fill(join(root, name))
  await pathInput.press('Enter')
  await dialog.getByRole('button', { name: '打开', exact: true }).click()
  await page.locator('textarea:enabled[placeholder="描述你想要构建的内容"]')
    .waitFor({ timeout: 15_000 })
}

/** Failure evidence goes to the gitignored .artifacts/ (repo convention). */
export async function saveFailureShot(page: Page, name: string): Promise<void> {
  const dir = fileURLToPath(new URL('../../../.artifacts', import.meta.url))
  mkdirSync(dir, { recursive: true })
  try {
    await page.screenshot({ path: `${dir}/${name}.png`, fullPage: true })
  } catch {
    // Best-effort evidence: a dead page/browser at failure time must not mask the real assertion error.
  }
}

/**
 * The conversation engine's Context key format, restated here rather than
 * imported: these specs live in the Host compiler aggregate, which must not
 * reach the Client plane. The engine's own copy is
 * `conversationContextKey` in dsh-client-runtime; a drift between them makes
 * the key miss its rendered node, so the assertion fails loudly.
 * @param kind - Definition kind.
 * @param id - Definition-local business identity.
 * @returns the engine-owned Context key.
 */
export function conversationContextKey(kind: string, id: string): string {
  return `${kind.length}:${kind}${id}`
}
