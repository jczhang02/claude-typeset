export type Variant = 'a' | 'b' | 'c' | 'd'

declare module 'claude-code' {
  interface PluginState {
    'typeset-status-proto': { open: Variant | null; isExpanded: boolean; isChecked: boolean }
  }
}
