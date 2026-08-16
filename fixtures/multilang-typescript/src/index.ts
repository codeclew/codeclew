import { formatValue, PrefixFormatter, readUnsafePayload } from "./format.js";

export function render(value: string): string {
  const formatter = new PrefixFormatter("value=");
  return ambientDecorate(formatValue(formatter, value));
}

export function renderLegacy(input: any): string {
  return render(readUnsafePayload(input));
}

export async function loadPlugin(moduleName: string): Promise<unknown> {
  return import(moduleName);
}
