// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! State machine module
//!
//! Provides the core state machine implementation and types for the VPN manager.

pub mod machine;
pub mod types;

pub use machine::{StateMachine, StateMachineConfig};
pub use types::{ActiveVpnInfo, Event, NmVpnState, TransitionReason, VpnState};
