export interface Formatter {
  format(value: string): string;
}

export class PrefixFormatter implements Formatter {
  constructor(private readonly prefix: string) {}

  format(value: string): string {
    return `${this.prefix}${value}`;
  }
}

export function formatValue(formatter: Formatter, value: string): string {
  return formatter.format(value);
}

export function selectAndFormat(
  key: string,
  formatters: Readonly<Record<string, Formatter>>,
  value: string,
): string {
  return formatters[key]?.format(value) ?? value;
}

export function readUnsafePayload(input: any): string {
  return String(input.payload);
}

