//! Native observer qualification contract.
//!
//! This is the single observer authority consumed by reconciliation. Platform
//! adapters may collect advisory evidence before qualification, but they can
//! only prove continuity through this trait and a durable end fence.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::reconcile::{ObserverEvent, ObserverQualification};
use super::{ExpectedScope, ProviderCapabilities};
use crate::error::Result;

#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ObserverFence {
    pub(crate) sequence: u64,
    pub(crate) durable_offset: u64,
    pub(crate) nonce: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObserverLease {
    pub(crate) owner_token: String,
    pub(crate) root_identity: Vec<u8>,
    pub(crate) provider_identity: Vec<u8>,
    pub(crate) policy_dependencies: Vec<PathBuf>,
    pub(crate) capabilities: ProviderCapabilities,
}

/// Generic `notify` delivery is useful as a reconciliation hint only. It has
/// no durable native cursor or linearizable fence and therefore can never
/// authorize a clean result.
pub(crate) struct AdvisoryObserver;

impl AdvisoryObserver {
    pub(crate) fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            durable_cursor: false,
            linearizable_fence: false,
            rename_pairing: false,
            overflow_scope: false,
            filesystem_supported: false,
            clean_proof_allowed: false,
            power_loss_durability: false,
        }
    }
}

pub(crate) trait QualifiedObserver: Send + Sync {
    fn begin_observation(&self, expected: &ExpectedScope) -> Result<ObserverFence>;

    fn end_fence(&self, expected: &ExpectedScope, start: &ObserverFence) -> Result<ObserverFence>;

    fn drain_through(
        &self,
        expected: &ExpectedScope,
        root_handle_identity: &[u8],
        start: &ObserverFence,
        end: &ObserverFence,
        sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
    ) -> Result<ObserverQualification>;

    /// Consume `(start, end]` while retaining `end` as the authenticated
    /// anchor for the next rotation.  Native adapters override this method;
    /// the default preserves compatibility for reconciliation-only test
    /// observers which do not provide continuous command authority.
    fn drain_through_retaining_end(
        &self,
        expected: &ExpectedScope,
        root_handle_identity: &[u8],
        start: &ObserverFence,
        end: &ObserverFence,
        sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
    ) -> Result<ObserverQualification> {
        self.drain_through(expected, root_handle_identity, start, end, sink)
    }

    fn rebind_retained_tail(
        &self,
        _previous: &ExpectedScope,
        _next: &ExpectedScope,
        _anchor: &ObserverFence,
    ) -> Result<()> {
        Err(crate::Error::DaemonUnavailable(
            "observer does not support retained-tail baseline rebinding".into(),
        ))
    }
}

pub(crate) enum SelectedObserver {
    #[cfg(target_os = "linux")]
    Linux,
    #[cfg(target_os = "macos")]
    MacOs,
    Advisory,
}

/// Platform selection does not activate ledger authority. Task 15 owns that
/// decision after both native qualification suites have passed.
pub(crate) fn select_observer() -> SelectedObserver {
    #[cfg(target_os = "linux")]
    {
        SelectedObserver::Linux
    }
    #[cfg(not(target_os = "linux"))]
    #[cfg(target_os = "macos")]
    {
        SelectedObserver::MacOs
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        SelectedObserver::Advisory
    }
}
