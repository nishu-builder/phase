import { describe, expect, it } from "vitest";

import type { SeatKind } from "../../multiplayer/seatTypes";
import { aiActorFromWaitingFor } from "../p2p-adapter";
import type { WaitingFor } from "../types";

// CR 732.2a: the host/P2P AI driver's actor gate must admit LoopShortcut (whose
// data field is `controller`, not `player`) into the engine-derived authorized-
// submitter path, so an AI-owned controller seat produces DeclareShortcut instead
// of hanging the game on an unhandled offer.

const seats: SeatKind[] = [
  { type: "HostHuman" },
  { type: "Ai", data: { difficulty: "medium", deck: { type: "Random" } } },
];

const loopShortcut: WaitingFor = {
  type: "LoopShortcut",
  data: {
    controller: 1,
    certificate: {
      unbounded: [{ DamageDealt: 1 }],
      win_kind: "LethalDamage",
      mandatory: false,
      residual_board_delta: { added: [], removed: [] },
    },
    schema: { iteration_count: "UntilLethal", points: [] },
  },
};

// A data-carrying state that carries neither `player` nor is LoopShortcut
// (AssistPayment routes on `caster`/`chosen`) — must return null. This is the
// non-vacuity control proving the admission is LoopShortcut-specific, not an
// always-return-authorizedSubmitter.
const assistPayment: WaitingFor = {
  type: "AssistPayment",
  data: { caster: 1, chosen: 0, max_generic: 0 },
};

describe("aiActorFromWaitingFor — LoopShortcut routing (T8)", () => {
  it("routes a LoopShortcut offer to the authorized submitter (controller)", () => {
    // Revert-probe target: delete `|| waitingFor.type === "LoopShortcut"` in
    // aiActorFromWaitingFor and this returns null instead of 1 → this fails.
    expect(aiActorFromWaitingFor(loopShortcut, seats, 1)).toBe(1);
  });

  it("does not blanket-admit non-player, non-LoopShortcut states (control)", () => {
    expect(aiActorFromWaitingFor(assistPayment, seats, 1)).toBeNull();
  });
});
