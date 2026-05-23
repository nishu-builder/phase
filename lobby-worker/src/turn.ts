// Mints short-lived Cloudflare Realtime TURN credentials on demand, so the
// client never ships static TURN credentials in its bundle (the prior Metered
// setup hardcoded them — anyone could extract and burn the quota).
//
// The client GETs /turn-credentials; this handler POSTs to Cloudflare's TURN
// API with the secret token and forwards the resulting ICE servers.
// Docs: https://developers.cloudflare.com/realtime/turn/generate-credentials/

export interface TurnEnv {
  /** Cloudflare Realtime TURN key ID (var, not secret). */
  TURN_KEY_ID?: string;
  /** Cloudflare Realtime TURN API token (secret: `wrangler secret put`). */
  TURN_KEY_API_TOKEN?: string;
  /** Credential lifetime in seconds (max 48h). Default 24h. */
  TURN_TTL_SECONDS?: string;
  /** Comma-separated origin allowlist, or "*" (default) to allow any. */
  ALLOWED_ORIGINS?: string;
}

function corsHeaders(request: Request, env: TurnEnv): Record<string, string> {
  const allow = (env.ALLOWED_ORIGINS ?? "*").trim();
  let allowOrigin = "*";
  if (allow !== "*") {
    const origin = request.headers.get("Origin") ?? "";
    const list = allow.split(",").map((s) => s.trim()).filter(Boolean);
    allowOrigin = list.includes(origin) ? origin : (list[0] ?? "");
  }
  return {
    "Access-Control-Allow-Origin": allowOrigin,
    "Access-Control-Allow-Methods": "GET, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type",
    Vary: "Origin",
  };
}

export async function handleTurnCredentials(
  request: Request,
  env: TurnEnv,
): Promise<Response> {
  const cors = corsHeaders(request, env);

  if (request.method === "OPTIONS") {
    return new Response(null, { status: 204, headers: cors });
  }

  if (!env.TURN_KEY_ID || !env.TURN_KEY_API_TOKEN) {
    // Not configured yet — the client falls back to STUN-only (direct/STUN
    // connections still work; symmetric-NAT peers won't relay until this is set).
    return Response.json(
      {
        error:
          "TURN not configured: set TURN_KEY_ID (var) and TURN_KEY_API_TOKEN (secret).",
      },
      { status: 503, headers: cors },
    );
  }

  const ttl = Number(env.TURN_TTL_SECONDS ?? "86400");
  const cfRes = await fetch(
    `https://rtc.live.cloudflare.com/v1/turn/keys/${env.TURN_KEY_ID}/credentials/generate-ice-servers`,
    {
      method: "POST",
      headers: {
        Authorization: `Bearer ${env.TURN_KEY_API_TOKEN}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ ttl }),
    },
  );

  if (!cfRes.ok) {
    return Response.json(
      { error: `TURN credential generation failed (${cfRes.status})` },
      { status: 502, headers: cors },
    );
  }

  // CF returns `{ iceServers: [ {stun}, {turn, username, credential} ] }` —
  // already an RTCIceServer[] the client drops straight into RTCConfiguration.
  const data = (await cfRes.json()) as { iceServers: unknown };
  return Response.json({ iceServers: data.iceServers }, { status: 200, headers: cors });
}
