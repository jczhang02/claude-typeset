// The typeset contract: the state the mod keeps for the session.

/** A drawn site's redraw count: bumped when a batch of pictures the site waited for lands. */
export type TypesetRedraw = number

/** Bumped when a health check lands and when the status band opens, closes or expands: the band reads it, so each redraws it. */
export type TypesetStatus = number

declare module 'claude-code' {
  interface PluginState {
    // One member per drawn site (a message's requestId); a reply reads its
    // own while drawing, so a write redraws that reply and no other.
    // The status band above the prompt reads `status` while drawing.
    typeset: { redraw: StateFamily<TypesetRedraw>; status: TypesetStatus }
  }
}
