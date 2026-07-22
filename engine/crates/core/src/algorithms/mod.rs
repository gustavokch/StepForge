//! Pattern-generation algorithms (Roll, Vary). Stubs — engine plan.
//! Invariants (architecture-spec §6.2/§6.3): neither alters `length`,
//! `midi_note`, or `speed_ratio`; both push a per-track undo snapshot first.

pub mod roll;
pub mod vary;
