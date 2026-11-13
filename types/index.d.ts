// The typeset contract: the state the mod keeps for the session.

/** A drawn site's redraw count: bumped when a batch of pictures the site waited for lands. */
export type TypesetRedraw = number

/** How many health checks have landed: the status pane reads it, so each one redraws the pane. */
export type TypesetStatus = number

declare module 'claude-code' {
  interface PluginState {
    // One member per drawn site (a message's requestId); a reply reads its
    // own while drawing, so a write redraws that reply and no other.
    // The status pane reads `status` while drawing.
    typeset: { redraw: StateFamily<TypesetRedraw>; status: TypesetStatus }
  }
}
