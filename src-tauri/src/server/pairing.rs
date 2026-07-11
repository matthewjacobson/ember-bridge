//! Browser pairing: hands the API token to a web origin after the user
//! clicks Approve in the desktop window (the "link button" pattern).
//!
//! Flow: the browser `POST /api/pair`s (no token), the desktop UI shows the
//! requesting origin with Approve/Deny, and the browser polls
//! `GET /api/pair/{id}` until it receives the token — released exactly once,
//! after which the request is consumed.
//!
//! Security invariants, enforced between here and [`super::routes`]:
//! * only allowlisted origins may create or poll a request;
//! * a request's result is only ever revealed to the origin that created it;
//! * the token is released once — a second poll finds nothing;
//! * one request may be pending at a time, and every request expires.
//!
//! Approval requires a human click in a window no web page can script, so a
//! local process forging an `Origin` header gains nothing it could not
//! already get by reading `config.json` off disk.

use serde::Serialize;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// How long a request may sit unanswered (or approved-but-unfetched).
pub const PAIRING_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pending,
    Approved,
    Denied,
}

#[derive(Debug)]
struct PairingRequest {
    id: String,
    origin: String,
    app_name: String,
    created_at: Instant,
    verdict: Verdict,
}

impl PairingRequest {
    fn expired(&self) -> bool {
        self.created_at.elapsed() > PAIRING_TTL
    }

    fn view(&self) -> PendingPairing {
        PendingPairing {
            id: self.id.clone(),
            origin: self.origin.clone(),
            app_name: self.app_name.clone(),
            expires_in_ms: PAIRING_TTL
                .saturating_sub(self.created_at.elapsed())
                .as_millis() as u64,
        }
    }
}

/// What the desktop UI renders in the approval banner (and what `POST
/// /api/pair` returns, minus the origin the caller already knows).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPairing {
    pub id: String,
    pub origin: String,
    pub app_name: String,
    pub expires_in_ms: u64,
}

/// Outcome of a browser poll of `GET /api/pair/{id}`.
#[derive(Debug, PartialEq, Eq)]
pub enum PollOutcome {
    /// Never existed, expired, or already consumed. Deliberately one bucket:
    /// a poller learns nothing about requests it did not create.
    Unknown,
    /// The id exists but belongs to a different origin.
    WrongOrigin,
    Pending,
    /// Terminal; the request is consumed by this poll.
    Denied,
    /// Terminal; the route layer attaches the token. Consumed by this poll.
    Approved,
}

/// The single pairing slot. One request at a time is a feature: the desktop
/// prompt is never a stack of dialogs, and a spamming page can only replace
/// its own expired/answered requests, not queue up dozens.
#[derive(Default)]
pub struct Pairing {
    slot: RwLock<Option<PairingRequest>>,
}

impl Pairing {
    /// Create a new request. `Err(())` if one is already pending.
    pub async fn begin(&self, origin: String, app_name: String) -> Result<PendingPairing, ()> {
        let mut slot = self.slot.write().await;
        if let Some(existing) = &*slot {
            if existing.verdict == Verdict::Pending && !existing.expired() {
                return Err(());
            }
        }
        let request = PairingRequest {
            id: random_id(),
            origin,
            app_name,
            created_at: Instant::now(),
            verdict: Verdict::Pending,
        };
        let view = request.view();
        *slot = Some(request);
        Ok(view)
    }

    /// Browser poll. Terminal outcomes (approved/denied) consume the slot,
    /// which is what makes the token single-release.
    pub async fn poll(&self, id: &str, origin: &str) -> PollOutcome {
        let mut slot = self.slot.write().await;
        let Some(request) = &*slot else {
            return PollOutcome::Unknown;
        };
        if request.expired() {
            *slot = None;
            return PollOutcome::Unknown;
        }
        if request.id != id {
            return PollOutcome::Unknown;
        }
        if request.origin != origin {
            return PollOutcome::WrongOrigin;
        }
        match request.verdict {
            Verdict::Pending => PollOutcome::Pending,
            Verdict::Denied => {
                *slot = None;
                PollOutcome::Denied
            }
            Verdict::Approved => {
                *slot = None;
                PollOutcome::Approved
            }
        }
    }

    /// The request the desktop UI should be showing, if any.
    pub async fn pending(&self) -> Option<PendingPairing> {
        let slot = self.slot.read().await;
        slot.as_ref()
            .filter(|r| r.verdict == Verdict::Pending && !r.expired())
            .map(|r| r.view())
    }

    /// Resolve the pending request (desktop UI only — the route is token
    /// gated). Returns the requesting origin, or `None` if `id` names no
    /// live pending request.
    pub async fn respond(&self, id: &str, approve: bool) -> Option<String> {
        let mut slot = self.slot.write().await;
        match &mut *slot {
            Some(r) if r.id == id && r.verdict == Verdict::Pending && !r.expired() => {
                r.verdict = if approve {
                    Verdict::Approved
                } else {
                    Verdict::Denied
                };
                Some(r.origin.clone())
            }
            _ => None,
        }
    }
}

/// 128 bits of randomness, hex-encoded — unguessable, and shaped so the
/// auth middleware can recognize `/api/pair/{id}` paths.
fn random_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..32)
        .map(|_| format!("{:x}", rng.random_range(0..16u8)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://ember.example";

    #[tokio::test]
    async fn approve_releases_once() {
        let pairing = Pairing::default();
        let req = pairing.begin(ORIGIN.into(), "Ember".into()).await.unwrap();

        assert_eq!(pairing.poll(&req.id, ORIGIN).await, PollOutcome::Pending);
        assert_eq!(pairing.respond(&req.id, true).await.as_deref(), Some(ORIGIN));
        assert_eq!(pairing.poll(&req.id, ORIGIN).await, PollOutcome::Approved);
        // Consumed: the same id yields nothing again.
        assert_eq!(pairing.poll(&req.id, ORIGIN).await, PollOutcome::Unknown);
        assert!(pairing.pending().await.is_none());
    }

    #[tokio::test]
    async fn deny_is_terminal() {
        let pairing = Pairing::default();
        let req = pairing.begin(ORIGIN.into(), "Ember".into()).await.unwrap();
        assert_eq!(pairing.respond(&req.id, false).await.as_deref(), Some(ORIGIN));
        assert_eq!(pairing.poll(&req.id, ORIGIN).await, PollOutcome::Denied);
        assert_eq!(pairing.poll(&req.id, ORIGIN).await, PollOutcome::Unknown);
        // A denied request cannot be flipped afterwards.
        assert_eq!(pairing.respond(&req.id, true).await, None);
    }

    #[tokio::test]
    async fn origin_is_checked_and_second_request_conflicts() {
        let pairing = Pairing::default();
        let req = pairing.begin(ORIGIN.into(), "Ember".into()).await.unwrap();
        assert_eq!(
            pairing.poll(&req.id, "https://other.example").await,
            PollOutcome::WrongOrigin
        );
        assert_eq!(pairing.poll("0000", ORIGIN).await, PollOutcome::Unknown);
        assert!(pairing.begin(ORIGIN.into(), "Ember".into()).await.is_err());
    }

    #[tokio::test]
    async fn expiry_clears_the_slot() {
        let pairing = Pairing::default();
        let req = pairing.begin(ORIGIN.into(), "Ember".into()).await.unwrap();
        // Backdate the request past its TTL.
        {
            let mut slot = pairing.slot.write().await;
            let r = slot.as_mut().unwrap();
            r.created_at = Instant::now() - (PAIRING_TTL + Duration::from_secs(1));
        }
        assert!(pairing.pending().await.is_none());
        assert_eq!(pairing.respond(&req.id, true).await, None);
        assert_eq!(pairing.poll(&req.id, ORIGIN).await, PollOutcome::Unknown);
        // The slot is free again for a new request.
        assert!(pairing.begin(ORIGIN.into(), "Ember".into()).await.is_ok());
    }
}
