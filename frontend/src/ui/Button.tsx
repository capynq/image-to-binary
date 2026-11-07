import type { ButtonHTMLAttributes, Ref } from 'react'

export type ButtonVariant = 'primary' | 'secondary' | 'quiet'

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  readonly variant?: ButtonVariant
  readonly wide?: boolean
  /** Work is in progress: exposed as `aria-busy` and the `is-busy` class; the label must say what is happening. */
  readonly busy?: boolean
  /** Forwarded to the underlying `button` element. */
  readonly ref?: Ref<HTMLButtonElement> | undefined
}

/**
 * Studio button. Every variant keeps the 44 px minimum target from `.button`. `busy` sets
 * `aria-busy` without adding `disabled`; callers that must block the action pass `disabled`
 * themselves, and that is the only way the control becomes disabled.
 */
export function Button({
  variant = 'secondary',
  wide = false,
  busy = false,
  className,
  type = 'button',
  ref,
  ...rest
}: ButtonProps) {
  const classes = [
    'button',
    `button-${variant}`,
    wide ? 'button-wide' : '',
    busy ? 'is-busy' : '',
    className ?? '',
  ]
    .filter((name) => name !== '')
    .join(' ')
  return (
    <button {...rest} ref={ref} type={type} className={classes} aria-busy={busy || undefined} />
  )
}
