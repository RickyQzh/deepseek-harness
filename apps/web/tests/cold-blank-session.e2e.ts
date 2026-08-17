/** Cold Session list visibility through the shipped compressed JSONL backend. */

import { mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'
import type { ChildProcess } from 'node:child_process'
import type { Browser, Page } from 'playwright'
import { chromium } from 'playwright'
import { afterAll, beforeAll, describe, expect, it, onTestFailed } from 'vitest'
import {
  captureStableAria, compareOrRefreshGolden, launchWebScaffold, seedBlankSession,
  watchConsole, webSnapshotMode, WELCOME_NOTICE_VERSION, type WebScaffold,
} from './scaffold.ts'
import {
  DIST_DIR, newEnglishPage, requireDist, rustWebRuntime, saveFailureShot,
  spawnRustDshWeb, stageWebClientPackages, stopRustChild,
} from './support.ts'

const SNAPSHOT_DIR = fileURLToPath(new URL('./snapshots/cold-blank-session', import.meta.url))
const SIDEBAR_EXPECTED = join(SNAPSHOT_DIR, 'sidebar.expected.md')
const MODE = webSnapshotMode()
const SESSION_ID = 'cold-blank-session-web-e2e'
const WORKSPACE_NAME = 'cold-blank-workspace'
const rust = rustWebRuntime()

describe('web e2e: cold blank Session visibility', () => {
  let scaffold: WebScaffold | undefined
  let rustChild: ChildProcess | undefined
  let rustTemps: string[] = []
  let browser: Browser | undefined
  let page: Page | undefined
  let tripwire: ReturnType<typeof watchConsole> | undefined
  let workspaceCwd: string
  let baseUrl: string

  beforeAll(async () => {
    if (rust) {
      requireDist()
      const workspaceRoot = await mkdtemp(join(tmpdir(), 'dsh-web-e2e-ws-'))
      const persistenceRoot = await mkdtemp(join(tmpdir(), 'dsh-web-e2e-sessions-'))
      const harnessHome = await mkdtemp(join(tmpdir(), 'dsh-web-e2e-home-'))
      rustTemps = [workspaceRoot, persistenceRoot, harnessHome]
      const cwd = join(workspaceRoot, WORKSPACE_NAME)
      await mkdir(cwd, { recursive: true })
      const sessionDir = join(persistenceRoot, SESSION_ID)
      await mkdir(sessionDir, { recursive: true })
      const createdAt = Date.now() - 60_000
      const jsonl = join(sessionDir, 'session.jsonl')
      await writeFile(jsonl, `${JSON.stringify({
        type: 'session',
        version: 0,
        id: SESSION_ID,
        createdAt,
        cwd,
        delegationDepth: 0,
      })}\n${JSON.stringify({
        type: 'session/end-seed',
        seq: 0,
        time: 0,
        data: {},
      })}\n`)
      expect((await stat(jsonl)).size).toBeLessThanOrEqual(1024)
      const settingsDir = join(harnessHome, 'settings')
      await mkdir(settingsDir, { recursive: true })
      await writeFile(join(settingsDir, 'ui-onboarding.json'), JSON.stringify({
        revision: 1,
        value: { welcomeNoticeVersion: WELCOME_NOTICE_VERSION },
      }))
      const clientPackages = await stageWebClientPackages()
      rustTemps.push(clientPackages)
      const spawned = await spawnRustDshWeb({
        dist: DIST_DIR,
        sessionRoot: persistenceRoot,
        cwd: workspaceRoot,
        home: harnessHome,
        clientPackages,
      })
      rustChild = spawned.child
      baseUrl = spawned.baseUrl
      workspaceCwd = workspaceRoot
    } else {
      scaffold = await launchWebScaffold({})
      const cwd = join(scaffold.workspaceCwd, WORKSPACE_NAME)
      await mkdir(cwd, { recursive: true })
      await seedBlankSession(scaffold, SESSION_ID, cwd)
      const header = (await scaffold.ctx.sessionPersistence.list())
        .find(candidate => candidate.id === SESSION_ID)
      if (header === undefined) throw new Error('blank Session fixture did not materialize')
      const location = scaffold.ctx.sessionPersistence.locate(header)
      if (location === undefined) throw new Error('JSONL fixture has no physical artifact')
      expect((await stat(location.path)).size).toBeLessThanOrEqual(1024)
      baseUrl = scaffold.baseUrl
      workspaceCwd = scaffold.workspaceCwd
    }

    browser = await chromium.launch()
    const launchedPage = await newEnglishPage(browser)
    page = launchedPage
    tripwire = watchConsole(launchedPage)
    await launchedPage.goto(baseUrl, { waitUntil: 'load' })
    try {
      await launchedPage.waitForSelector('[class*="frame"]', { timeout: rust ? 90_000 : 30_000 })
    } catch (error) {
      await saveFailureShot(launchedPage, 'web-e2e-cold-blank-session-boot')
      const body = await launchedPage.locator('body').innerText().catch(() => '')
      throw new Error(`SPA did not reach layout frame. body=${body.slice(0, 2000)}`, { cause: error })
    }
  }, 180_000)

  afterAll(async () => {
    await browser?.close()
    await scaffold?.close()
    await stopRustChild(rustChild)
    await Promise.all(rustTemps.map(dir => rm(dir, { recursive: true, force: true })))
  })

  it('keeps the verified cold blank Session out of the sidebar', async () => {
    if (page === undefined || tripwire === undefined) {
      throw new Error('web e2e page was not launched')
    }
    const launched = page
    const errors = tripwire
    onTestFailed(() => saveFailureShot(launched, 'web-e2e-cold-blank-session'))
    const tree = launched.getByRole('tree', { name: 'Sessions' })
    await tree.waitFor({ timeout: 30_000 })
    expect(await tree.getByText(WORKSPACE_NAME, { exact: true }).count()).toBe(0)
    if (!rust) {
      const sidebar = await captureStableAria(launched, '[role="tree"][aria-label="Sessions"]', workspaceCwd)
      await compareOrRefreshGolden(SIDEBAR_EXPECTED, sidebar, MODE)
    }
    expect(errors.pageErrors).toEqual([])
  })
})
