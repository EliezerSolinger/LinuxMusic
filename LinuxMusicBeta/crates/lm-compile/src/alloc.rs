//! The only allocator of node indices and parameter slots (I-5.3, S6).
//!
//! An index or a slot goes through three states: free, used, and **pending**: taken out by an install that has not
//! come back yet. It is free again only when the [`lm_render::Returned`] of that install arrives (for nodes, the
//! `freed` list the renderer fills; for slots, by the install's generation). Reusing an index bumps its generation,
//! so a message about the old occupant can never be taken for the new one. The lowest free number is always taken
//! first (deterministic programs).

use std::collections::{BTreeMap, BTreeSet};

use lm_base::{Error, Result};
use lm_render::{NodeIndex, ParamSlot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    Used,
    Pending,
}

/// The node table's indices
#[derive(Clone, Debug)]
pub(crate) struct NodeAllocator {
    state: Vec<State>,
    generation: Vec<u32>,
    free: BTreeSet<u32>,
}

impl NodeAllocator {
    pub(crate) fn new(max_nodes: usize) -> Self {
        let max = u32::try_from(max_nodes).unwrap_or(u32::MAX);
        Self { state: vec![State::Free; max as usize], generation: vec![0; max as usize], free: (0..max).collect() }
    }

    pub(crate) fn free_count(&self) -> usize {
        self.free.len()
    }

    pub(crate) fn pending_count(&self) -> usize {
        self.state.iter().filter(|s| **s == State::Pending).count()
    }

    /// The lowest free index, with its new generation. Key `compile.limits.nodes_pending` when only pending ones
    /// are left (the audio side has not returned them yet; the next compile after the return succeeds)
    pub(crate) fn alloc(&mut self) -> Result<NodeIndex> {
        let Some(i) = self.free.pop_first() else {
            return Err(Error::new("compile.limits.nodes_pending").arg("pending", self.pending_count() as i64));
        };
        self.state[i as usize] = State::Used;
        self.generation[i as usize] = self.generation[i as usize].wrapping_add(1);
        Ok(NodeIndex::new(i, self.generation[i as usize]))
    }

    /// Marks a used index as taken out by an install on its way
    pub(crate) fn release(&mut self, node: NodeIndex) {
        if let Some(s) = self.state.get_mut(node.index as usize)
            && *s == State::Used
            && self.generation[node.index as usize] == node.generation
        {
            *s = State::Pending;
        }
    }

    /// The renderer gave the index back (S6)
    pub(crate) fn freed(&mut self, node: NodeIndex) {
        let i = node.index as usize;
        if self.state.get(i) == Some(&State::Pending) && self.generation[i] == node.generation {
            self.state[i] = State::Free;
            self.free.insert(node.index);
        }
    }

    /// Rebuilding from a table (H14): `used` are the live indices, `pending` the ones on their way back
    pub(crate) fn restore(max_nodes: usize, used: &[NodeIndex], pending: &[NodeIndex]) -> Self {
        let mut a = Self::new(max_nodes);
        for (list, st) in [(used, State::Used), (pending, State::Pending)] {
            for n in list {
                let i = n.index as usize;
                if i < a.state.len() {
                    a.state[i] = st;
                    a.generation[i] = n.generation;
                    a.free.remove(&n.index);
                }
            }
        }
        a
    }

    pub(crate) fn pending(&self) -> Vec<NodeIndex> {
        self.state
            .iter()
            .enumerate()
            .filter(|(_, s)| **s == State::Pending)
            .map(|(i, _)| NodeIndex::new(i as u32, self.generation[i]))
            .collect()
    }
}

/// The parameter slots
#[derive(Clone, Debug)]
pub(crate) struct SlotAllocator {
    free: BTreeSet<u32>,
    /// By the generation of the install that unbound them
    pending: BTreeMap<u64, Vec<u32>>,
}

impl SlotAllocator {
    pub(crate) fn new(max_params: usize) -> Self {
        let max = u32::try_from(max_params).unwrap_or(u32::MAX);
        Self { free: (0..max).collect(), pending: BTreeMap::new() }
    }

    pub(crate) fn free_count(&self) -> usize {
        self.free.len()
    }

    pub(crate) fn alloc(&mut self) -> Result<ParamSlot> {
        match self.free.pop_first() {
            Some(s) => Ok(ParamSlot(s)),
            None => Err(Error::new("compile.limits.params_pending")
                .arg("pending", self.pending.values().map(Vec::len).sum::<usize>() as i64)),
        }
    }

    /// The program of `generation` no longer binds `slot`
    pub(crate) fn release(&mut self, slot: ParamSlot, generation: u64) {
        self.pending.entry(generation).or_default().push(slot.0);
    }

    /// The install of `generation` came back: everything it unbound is free (and anything older, which came back
    /// before it since the renderer returns installs in order)
    pub(crate) fn returned(&mut self, generation: u64) {
        let later = self.pending.split_off(&(generation + 1));
        for (_, slots) in std::mem::replace(&mut self.pending, later) {
            self.free.extend(slots);
        }
    }

    pub(crate) fn restore(max_params: usize, used: &[ParamSlot], pending: &[(u64, ParamSlot)]) -> Self {
        let mut a = Self::new(max_params);
        for s in used {
            a.free.remove(&s.0);
        }
        for (g, s) in pending {
            if a.free.remove(&s.0) {
                a.pending.entry(*g).or_default().push(s.0);
            }
        }
        a
    }

    pub(crate) fn pending(&self) -> Vec<(u64, ParamSlot)> {
        self.pending.iter().flat_map(|(g, v)| v.iter().map(move |s| (*g, ParamSlot(*s)))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_wait_for_their_return() {
        let mut a = NodeAllocator::new(2);
        let n0 = a.alloc().unwrap();
        let n1 = a.alloc().unwrap();
        assert_eq!((n0.index, n1.index), (0, 1));
        assert_eq!(a.alloc().unwrap_err().key(), "compile.limits.nodes_pending");
        a.release(n0);
        assert!(a.alloc().is_err(), "pending is not free");
        a.freed(n0);
        let again = a.alloc().unwrap();
        assert_eq!(again.index, 0);
        assert_ne!(again.generation, n0.generation);
        // A stale return of an older occupant changes nothing
        a.freed(n0);
        assert!(a.alloc().is_err());
    }

    #[test]
    fn slots_wait_for_the_generation() {
        let mut a = SlotAllocator::new(1);
        let s = a.alloc().unwrap();
        a.release(s, 5);
        assert!(a.alloc().is_err());
        a.returned(4);
        assert!(a.alloc().is_err());
        a.returned(5);
        assert_eq!(a.alloc().unwrap(), s);
    }
}
