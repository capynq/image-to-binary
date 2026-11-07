declare module 'heif-decoder-glue' {
  import type { HeifModule } from '../heifDecoderCore'

  /** Resolved by vite.config.ts to the built Emscripten glue, or to the unavailable stub. */
  export default function createHeifDecoderModule(): Promise<HeifModule>
}
