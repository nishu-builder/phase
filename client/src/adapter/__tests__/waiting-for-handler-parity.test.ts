import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import type { WaitingFor } from "../types";
import { HANDLED_WAITING_FOR_TYPES } from "../../game/waitingForRegistry";
import { repoRoot, rustEnumVariants } from "./rustEnumVariants";

/**
 * Engine `WaitingFor` variants that are never surfaced to a human and so
 * legitimately have no frontend UI handler.
 *
 * This set is intentionally EMPTY. An audit of `WaitingFor::acting_player()`
 * (crates/engine/src/types/game_state.rs) shows every variant routes its
 * authorization to a human player — single-pending mulligan variants resolve
 * to the one pending player, `VoteChoice` to `actor.resolve(player)`,
 * `AssistPayment` to `chosen`, and every remaining variant to `Some(*player)`.
 * The sole exception is `GameOver`, which returns `None` (terminal lifecycle
 * state) and is already present in `HANDLED_WAITING_FOR_TYPES`. No
 * internal-only, never-player-facing variant exists today.
 *
 * If a future variant is genuinely never player-facing, add it here WITH a
 * cited `acting_player()` reference proving it returns `None`.
 */
const INTERNAL_NEVER_PLAYER_FACING: ReadonlySet<WaitingFor["type"]> =
  new Set<WaitingFor["type"]>([]);

/**
 * Engine `WaitingFor` variants whose real player-facing modal ships in Phase 5 of the
 * PR-7 combo-detector series. `Interactive` loop-detection is opt-in / default-Off, so
 * no human reaches these states in Phase 3 (engine/AI/tests only). Until the Phase-5
 * modal lands, `UnhandledWaitingForModal` is the fail-loud runtime net. When Phase 5
 * wires the modal, MOVE these into `HANDLED_WAITING_FOR_TYPES` (their render sites then
 * satisfy waiting-for-handler-dispatch-coverage.test.ts) and delete this entry.
 *
 * They cannot honestly go in `INTERNAL_NEVER_PLAYER_FACING`: both have
 * `acting_player() == Some(..)` (`LoopShortcut → Some(controller)`,
 * `RespondToShortcut → Some(player)`), whereas that set's contract requires
 * `acting_player() == None`.
 */
const PENDING_MODAL_PHASE5: ReadonlySet<WaitingFor["type"]> =
  new Set<WaitingFor["type"]>([]);

describe("WaitingFor handler parity", () => {
  it("every engine WaitingFor variant has a frontend UI handler", () => {
    const rustSource = readFileSync(
      resolve(repoRoot(), "crates/engine/src/types/game_state.rs"),
      "utf8",
    );
    const engineVariants = rustEnumVariants(rustSource, "WaitingFor");

    const unhandled = engineVariants.filter(
      (variant) =>
        !HANDLED_WAITING_FOR_TYPES.has(variant as WaitingFor["type"]) &&
        !INTERNAL_NEVER_PLAYER_FACING.has(variant as WaitingFor["type"]) &&
        !PENDING_MODAL_PHASE5.has(variant as WaitingFor["type"]),
    );

    expect(
      unhandled,
      `Engine WaitingFor variant(s) [${unhandled.join(", ")}] have no frontend UI handler. ` +
        "Add a handler to HANDLED_WAITING_FOR_TYPES (client/src/game/waitingForRegistry.ts) " +
        "and wire a corresponding modal/overlay in GamePage. Only if the variant is truly " +
        "internal and never surfaced to a player, add it to INTERNAL_NEVER_PLAYER_FACING with " +
        "a cited acting_player() reference proving it returns None.",
    ).toEqual([]);
  });

  it("has no stale INTERNAL_NEVER_PLAYER_FACING allowlist entries", () => {
    const rustSource = readFileSync(
      resolve(repoRoot(), "crates/engine/src/types/game_state.rs"),
      "utf8",
    );
    const engineVariants = new Set(rustEnumVariants(rustSource, "WaitingFor"));

    const stale = [...INTERNAL_NEVER_PLAYER_FACING].filter(
      (variant) => !engineVariants.has(variant),
    );

    expect(
      stale,
      `INTERNAL_NEVER_PLAYER_FACING contains entries [${stale.join(", ")}] that no longer ` +
        "exist on the engine WaitingFor enum. Remove the stale allowlist entries.",
    ).toEqual([]);
  });

  it("has no stale PENDING_MODAL_PHASE5 allowlist entries", () => {
    const rustSource = readFileSync(
      resolve(repoRoot(), "crates/engine/src/types/game_state.rs"),
      "utf8",
    );
    const engineVariants = new Set(rustEnumVariants(rustSource, "WaitingFor"));

    const stale = [...PENDING_MODAL_PHASE5].filter(
      (variant) => !engineVariants.has(variant),
    );

    expect(
      stale,
      `PENDING_MODAL_PHASE5 contains entries [${stale.join(", ")}] that no longer exist on ` +
        "the engine WaitingFor enum. Either the variant was renamed/removed (update the " +
        "allowlist) or its Phase-5 modal landed (move it to HANDLED_WAITING_FOR_TYPES).",
    ).toEqual([]);
  });
});
