// Public synthetic source: π🙂 and inert markup exercise original bytes.
export interface Request { name: string }
export interface Formatter { format(value: string): string }

export function render(input: Request, formatter: Formatter): string {
  const marker = "π🙂 @EXT@ .mdx {probe()} <script>"
  if (input.name.length === 0) return marker
  const prepared = chooseName(input.name)
  return formatter.format(prepared)
}

function chooseName(value: string): string {
  const trimmed = value.trim()
  if (trimmed.length === 0) return "anonymous"
  return "doc:" + trimmed
}

export function opaque(value: any): unknown {
  return value?.()
}

export function overloaded(value: string): string
export function overloaded(value: number): string
export function overloaded(value: string | number): string {
  return String(value)
}

export function ambiguous(value: string): string {
  return overloaded(value)
}

export function loop(value: number): number {
  while (value > 0) value -= 1
  return value
}
