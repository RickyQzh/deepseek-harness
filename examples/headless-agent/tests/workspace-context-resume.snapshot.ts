/**
 * Assembled-app regression for persisted workspace-instruction resume state.
 * @module workspace-context-resume-snapshot
 */

import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { Context } from '@deepseek-ai/cordis'
import { normalizeSessionLog, scrubRequestHeaders, type NormalizeContext } from '@deepseek-ai/dsh-acp-snapshot'
import { LOADER_SMOKE_TEST_TIMEOUT_MS, runLoaderSmoke } from '@deepseek-ai/dsh-loader-smoke'
import { createUserMessage } from '@deepseek-ai/dsh-llm'
import SessionStore, {
  SESSION_FORMAT_VERSION,
  SessionId,
  type SessionEvent,
  type SessionHeader,
} from '@deepseek-ai/dsh-session'
import JsonlSessionPersistence from '@deepseek-ai/dsh-session-persistence-jsonl'
import { renderWorkspaceContext } from '@deepseek-ai/dsh-agent-instructions'
import { resolveConfig, workspaceBaselineIdentity } from '@deepseek-ai/dsh-agent-instructions/src/config.ts'
import { describe, expect, it } from 'vitest'

const fixtureDir = join(dirname(fileURLToPath(import.meta.url)), 'workspace-context-resume-snapshots/offline-edit')
const replayFixture = join(fixtureDir, 'replay.jsonl')
const replayOverride = join(fixtureDir, 'replay.override.json')
const sessionExpected = join(fixtureDir, 'session.expected.jsonl')
const precedenceExpected = join(dirname(fixtureDir), 'precedence-change/session.expected.jsonl')
const configPath = fileURLToPath(new URL('../workspace-context-resume.cordis.snapshot.yml', import.meta.url))
const rustConfigPath = fileURLToPath(new URL('../rust.workspace-context-resume.cordis.yml', import.meta.url))
const binScript = fileURLToPath(new URL('./fixtures/headless-driver.ts', import.meta.url))
const tsconfigPath = fileURLToPath(new URL('../../../tsconfig.json', import.meta.url))
const rustBinDefault = fileURLToPath(new URL('../../../target/debug/dsh', import.meta.url))
const rustRuntime = process.env.DSH_RUNTIME === 'rust'
const rustBin = process.env.DSH_RUNTIME_BIN && process.env.DSH_RUNTIME_BIN !== ''
  ? process.env.DSH_RUNTIME_BIN
  : rustBinDefault
const rustSmokeTimeoutMs = 120_000
const prompt = 'Acknowledge the current workspace instruction.'
const sessionId = SessionId('workspace-context-resume')
const rustPersistRelative = join('.sessions', 'workspace-context-resume', 'session.jsonl')
const refreshing = process.env.DSH_SNAPSHOT === 'refresh'
const oldInstruction = 'Old workspace instruction.'
const newInstruction = 'New workspace instruction after offline edit.'

interface SeedBaselineOptions {
  files?: Array<{ name: string; content: string }>
  instructionFileCandidates?: string[]
}

async function seedVisibleBaseline(
  root: string,
  cwd: string,
  options: SeedBaselineOptions = {},
): Promise<string> {
  const ctx = new Context()
  await ctx.plugin(SessionStore)
  await ctx.plugin(JsonlSessionPersistence, { root, compression: 'none' })
  const meta: SessionHeader = {
    version: SESSION_FORMAT_VERSION,
    id: sessionId,
    createdAt: 1,
    cwd,
    delegationDepth: 0,
  }
  const files = options.files ?? [{ name: 'AGENTS.md', content: oldInstruction }]
  const baseline = renderWorkspaceContext(files.map(file => ({
    absolutePath: join(cwd, file.name),
    displayPath: file.name,
    content: file.content,
  })), { maxBytes: 65536 })
  const config = resolveConfig({
    dshHome: join(cwd, '.dsh'),
    maxBytes: 65536,
    ...options.instructionFileCandidates === undefined
      ? {}
      : { instructionFileCandidates: options.instructionFileCandidates },
  })
  const events: SessionEvent[] = [
    { type: 'turn/start', seq: 0, time: 10, data: { turn: 1 } },
    {
      type: 'user/message',
      seq: 1,
      time: 11,
      data: createUserMessage({ content: [{ type: 'text', text: 'Remember the workspace instruction.' }], source: { kind: 'user' } }),
      surfaceOp: 'append',
    },
    {
      type: 'user/message',
      seq: 2,
      time: 12,
      data: createUserMessage({
        content: [{ type: 'text', text: baseline.text }],
        source: {
          kind: 'agent-instructions',
          form: 'instructions',
          baseline: true,
          baselineIdentity: workspaceBaselineIdentity(config, cwd, cwd),
          changes: files.map(file => ({
            action: 'set',
            scope: `.\0${file.name}`,
            path: file.name,
            digest: createHash('sha1').update(file.content).digest('hex'),
          })),
        },
      }),
      surfaceOp: 'append',
    },
    { type: 'turn/end', seq: 3, time: 13, data: { turn: 1, reason: { kind: 'completed' } } },
  ]
  try {
    await ctx.sessionPersistence.create(meta)
    await ctx.sessionPersistence.append(sessionId, events)
    const location = ctx.sessionPersistence.locate(meta)
    if (location === undefined) throw new Error('JSONL backend did not locate the seeded session')
    return location.path
  }   finally {
    await ctx.fiber.dispose()
  }
}

function requireRustBin(): string {
  if (!existsSync(rustBin)) {
    throw new Error(`DSH_RUNTIME=rust but ${rustBin} is missing; run cargo build -p dsh-cli`)
  }
  return rustBin
}

function rustNamedTimeout(): number {
  return rustRuntime ? rustSmokeTimeoutMs : LOADER_SMOKE_TEST_TIMEOUT_MS
}

function resolveResumeLaunch(): {
  launch?: { command: string; args: string[] }
  env: NodeJS.ProcessEnv
} {
  if (!rustRuntime) return { env: {} }
  return {
    launch: { command: requireRustBin(), args: ['--profile', 'headless', prompt] },
    env: { DSH_CORDIS_CONFIG: rustConfigPath },
  }
}

function rustResumeReplayFifo(): string {
  const text = 'RESUME_DONE'
  const header = JSON.stringify({
    type: 'session',
    version: 0,
    id: 'rust-replay',
    createdAt: 0,
    delegationDepth: 0,
  })
  const chunks = [
    { type: 'block-start', index: 0, blockType: 'text' },
    { type: 'text-delta', index: 0, text },
    { type: 'block-end', index: 0, block: { type: 'text', text } },
    { type: 'finish', reason: { kind: 'stop' } },
  ]
  const events = chunks.map((chunk, seq) => JSON.stringify({
    type: 'assistant/chunk',
    seq,
    time: 0,
    data: { turn: 1, step: 1, chunk },
  }))
  return `${[header, ...events].join('\n')}\n`
}

async function seedRustResumeSession(cwd: string): Promise<void> {
  const dir = join(cwd, '.sessions', 'workspace-context-resume')
  await mkdir(dir, { recursive: true })
  await writeFile(join(dir, 'session.jsonl'), `${JSON.stringify({
    type: 'session',
    version: SESSION_FORMAT_VERSION,
    id: sessionId,
    createdAt: 1,
    cwd,
    delegationDepth: 0,
  })}\n`)
}

function persistHasAgentInstructions(content: string): boolean {
  return content.trimEnd().split('\n').some((line) => {
    const record = JSON.parse(line) as {
      type?: string
      data?: { source?: { kind?: string } }
    }
    return record.type === 'user/message' && record.data?.source?.kind === 'agent-instructions'
  })
}

function expectResumeStderr(stderr: string): void {
  if (!rustRuntime) {
    expect(stderr).toBe('')
    return
  }
  expect(stderr.replace(/^skill-filesystem: skipping missing skill root .+\n/gm, '')).toBe('')
}

describe('agent-instructions resume snapshot', () => {
  it('appends an offline replacement without duplicating the visible baseline', async () => {
    let cwd = ''
    let sessionPath = ''
    const rustLaunch = resolveResumeLaunch()
    const result = await runLoaderSmoke({
      label: 'agent-instructions resume headless stream-json snapshot',
      tempDirPrefix: 'dsh-workspace-context-resume-',
      binScript,
      libBinScript: binScript,
      configPath,
      binArgs: [configPath, prompt],
      tsconfigPath,
      processTimeoutMs: rustRuntime ? rustSmokeTimeoutMs : undefined,
      ...rustLaunch.launch === undefined ? {} : { launch: rustLaunch.launch },
      env: {
        DSH_SNAPSHOT_FILE: rustRuntime ? 'replay.jsonl' : replayFixture,
        DSH_SNAPSHOT_OVERRIDE: replayOverride,
        ...rustLaunch.env,
      },
      prepare: async (runCwd) => {
        cwd = runCwd
        await mkdir(join(runCwd, '.git'), { recursive: true })
        await writeFile(join(runCwd, 'AGENTS.md'), `${newInstruction}\n`)
        if (rustRuntime) {
          await seedRustResumeSession(runCwd)
          await writeFile(join(runCwd, 'replay.jsonl'), rustResumeReplayFifo())
          return
        }
        sessionPath = await seedVisibleBaseline(join(runCwd, '.sessions'), runCwd)
      },
      inspect: async () => {
        if (rustRuntime) {
          const persist = await readFile(join(cwd, rustPersistRelative), 'utf8')
          expect(persistHasAgentInstructions(persist)).toBe(true)
          return
        }
        const normalization: NormalizeContext = { sessionIds: [sessionId], cwd }
        const session = scrubRequestHeaders(normalizeSessionLog(await readFile(sessionPath, 'utf8'), normalization))
        if (refreshing) await writeFile(sessionExpected, session)
        expect(session).toBe(await readFile(sessionExpected, 'utf8'))

        const records = session.trimEnd().split('\n').map(line => JSON.parse(line) as {
          type?: string
          data?: {
            source?: { kind?: string; baseline?: boolean; changes?: Array<Record<string, unknown>> }
            content?: Array<{ type?: string; text?: string }>
          }
        })
        const workspaceEvents = records.filter(record => record.type === 'user/message'
          && record.data?.source?.kind === 'agent-instructions')
        expect(workspaceEvents.filter(record => record.data?.source?.baseline === true)).toHaveLength(1)
        expect(workspaceEvents.filter(record => record.data?.source?.baseline !== true)).toHaveLength(1)
        expect(workspaceEvents.at(-1)?.data?.source?.changes).toMatchObject([{
          action: 'replace', scope: '.\0AGENTS.md', path: 'AGENTS.md',
        }])
        expect(JSON.stringify(workspaceEvents.at(-1)?.data?.content)).toContain(newInstruction)

        const files = await readdir(join(cwd, '.sessions'), { recursive: true })
        expect(files.filter(file => file.endsWith('.jsonl'))).toHaveLength(1)
      },
    })

    expectResumeStderr(result.stderr)
    if (rustRuntime) {
      expect(result.stdout).toBe('RESUME_DONE\n')
      return
    }
    const records = result.stdout.trimEnd().split('\n').map(line => JSON.parse(line) as Record<string, unknown>)
    expect(records.at(-1)).toMatchObject({
      type: 'result',
      sessionId,
      output: 'RESUME_DONE',
    })
  }, rustNamedTimeout())

  it('supersedes an incompatible baseline when precedence changed offline', async () => {
    let cwd = ''
    let sessionPath = ''
    const rustLaunch = resolveResumeLaunch()
    const result = await runLoaderSmoke({
      label: 'agent-instructions precedence-change resume snapshot',
      tempDirPrefix: 'dsh-workspace-context-precedence-',
      binScript,
      libBinScript: binScript,
      configPath,
      binArgs: [configPath, prompt],
      tsconfigPath,
      processTimeoutMs: rustRuntime ? rustSmokeTimeoutMs : undefined,
      ...rustLaunch.launch === undefined ? {} : { launch: rustLaunch.launch },
      env: {
        DSH_SNAPSHOT_FILE: rustRuntime ? 'replay.jsonl' : replayFixture,
        DSH_SNAPSHOT_OVERRIDE: replayOverride,
        ...rustLaunch.env,
      },
      prepare: async (runCwd) => {
        cwd = runCwd
        await mkdir(join(runCwd, '.git'), { recursive: true })
        await writeFile(join(runCwd, 'AGENTS.md'), 'Current AGENTS rule.\n')
        await writeFile(join(runCwd, 'CLAUDE.md'), 'Current CLAUDE rule.\n')
        if (rustRuntime) {
          await seedRustResumeSession(runCwd)
          await writeFile(join(runCwd, 'replay.jsonl'), rustResumeReplayFifo())
          return
        }
        sessionPath = await seedVisibleBaseline(join(runCwd, '.sessions'), runCwd, {
          files: [
            { name: 'CLAUDE.md', content: 'Old CLAUDE rule.' },
            { name: 'AGENTS.md', content: 'Old AGENTS rule.' },
          ],
          instructionFileCandidates: ['CLAUDE.md', 'AGENTS.md'],
        })
      },
      inspect: async () => {
        if (rustRuntime) {
          const persist = await readFile(join(cwd, rustPersistRelative), 'utf8')
          expect(persistHasAgentInstructions(persist)).toBe(true)
          return
        }
        const normalization: NormalizeContext = { sessionIds: [sessionId], cwd }
        const session = scrubRequestHeaders(normalizeSessionLog(await readFile(sessionPath, 'utf8'), normalization))
        if (refreshing) {
          await mkdir(dirname(precedenceExpected), { recursive: true })
          await writeFile(precedenceExpected, session)
        }
        expect(session).toBe(await readFile(precedenceExpected, 'utf8'))

        const records = session.trimEnd().split('\n').map(line => JSON.parse(line) as {
          type?: string
          data?: {
            source?: { kind?: string; baseline?: boolean }
            content?: Array<{ type?: string; text?: string }>
          }
        })
        const baselines = records.filter(record => record.type === 'user/message'
          && record.data?.source?.kind === 'agent-instructions'
          && record.data.source.baseline === true)
        expect(baselines).toHaveLength(2)
        const replacement = JSON.stringify(baselines.at(-1)?.data?.content)
        expect(replacement).toContain('replaces all earlier workspace instruction baselines')
        expect(replacement.indexOf('Instructions from: AGENTS.md'))
          .toBeLessThan(replacement.indexOf('Instructions from: CLAUDE.md'))
      },
    })

    expectResumeStderr(result.stderr)
    if (rustRuntime) {
      expect(result.stdout).toBe('RESUME_DONE\n')
      return
    }
    expect(result.stdout.trimEnd().split('\n').map(line => JSON.parse(line) as Record<string, unknown>).at(-1))
      .toMatchObject({
        type: 'result',
        sessionId,
        output: 'RESUME_DONE',
      })
  }, rustNamedTimeout())
})
