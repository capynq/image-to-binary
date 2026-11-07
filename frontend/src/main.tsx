import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import { createBrowserRuntime, isNativeShell } from './runtime'
import type { AppRuntime } from './runtime'
import './styles.css'

const root = document.getElementById('root')

if (!root) {
  throw new Error('The application root is missing.')
}

const studioRoot = createRoot(root)

function show({ controller, native, menu }: AppRuntime): void {
  studioRoot.render(
    <StrictMode>
      <App controller={controller} native={native} menu={menu ?? null} />
    </StrictMode>,
  )
}

function fail(error: unknown): void {
  studioRoot.render(
    <p role="alert">
      {error instanceof Error ? error.message : 'The application could not start.'}
    </p>,
  )
}

if (isNativeShell()) {
  import('./nativeRuntime')
    .then(({ createNativeRuntime }) => show(createNativeRuntime()))
    .catch(fail)
} else {
  show(createBrowserRuntime())
}
