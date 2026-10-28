//! Disjoint sets over indices (union-find): `parent` starts as `0..n`, so each index is its own
//! set (`parent[i] == i` for a root). [`union`] joins two sets and [`find`] names the set an index
//! is in; two indices are in one set when they have the same root.

/// The root of `i`'s set; each index on the way is pointed at its grandparent (path splitting).
/// An index outside `parent` is its own root.
pub fn find(parent: &mut [usize], mut i: usize) -> usize {
    while let Some(&p) = parent.get(i) {
        if p == i {
            break;
        }
        let gp = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = gp;
        }
        i = p;
    }
    i
}

/// Join the sets of `a` and `b` (`b`'s root becomes the root of both). True when they were two
/// sets.
pub fn union(parent: &mut [usize], a: usize, b: usize) -> bool {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra == rb {
        return false;
    }
    if let Some(slot) = parent.get_mut(ra) {
        *slot = rb;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_and_halves_paths() {
        let mut p: Vec<usize> = (0..6).collect();
        assert!(union(&mut p, 0, 1));
        assert!(union(&mut p, 1, 2));
        assert!(union(&mut p, 4, 5));
        assert!(!union(&mut p, 2, 0), "already one set");
        assert_eq!(find(&mut p, 0), find(&mut p, 2));
        assert_ne!(find(&mut p, 0), find(&mut p, 4));
        assert_eq!(find(&mut p, 3), 3);
        assert_eq!(find(&mut p, 99), 99);
        // A look-up points each index on its path at its grandparent, and `b`'s root becomes the
        // root of both.
        let mut p = vec![1, 2, 3, 4, 4, 5];
        assert!(union(&mut p, 5, 0));
        assert_eq!(p, [2, 3, 4, 4, 4, 4]);
        assert_eq!(find(&mut p, 0), 4);
        assert_eq!(p, [4, 3, 4, 4, 4, 4]);
    }
}
