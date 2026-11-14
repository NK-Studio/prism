//! Copy-on-write pixel buffers for brush assets.
//!
//! A brush is cloned many times a frame (options bar, Brush Settings, Tool Property, previews,
//! the live stroke) and compared to detect edits. Imported CLIP STUDIO brushes carry tips and
//! colour planes of tens of megabytes, so cloning and comparing them by value made every UI frame
//! copy and scan those bytes. [`Shared`] keeps the buffer behind an `Arc`: clones share it,
//! equality short-cuts on shared storage, and the first write copies it (`Arc::make_mut`).

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// A `Vec<T>` whose clones share their storage until one of them is written.
#[derive(Default)]
pub struct Shared<T>(Arc<Vec<T>>);

impl<T> Shared<T> {
    pub fn new(v: Vec<T>) -> Self {
        Self(Arc::new(v))
    }

    /// Whether `self` and `other` share one buffer (no comparison of the contents).
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T: Clone> Shared<T> {
    /// The owned vector (copied only when other clones still share it).
    pub fn into_vec(self) -> Vec<T> {
        Arc::try_unwrap(self.0).unwrap_or_else(|shared| (*shared).clone())
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Deref for Shared<T> {
    type Target = Vec<T>;
    fn deref(&self) -> &Vec<T> {
        &self.0
    }
}

impl<T: Clone> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        Arc::make_mut(&mut self.0)
    }
}

impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || *self.0 == *other.0
    }
}

impl<T: PartialEq> PartialEq<Vec<T>> for Shared<T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        *self.0 == *other
    }
}

impl<T: PartialEq, const N: usize> PartialEq<[T; N]> for Shared<T> {
    fn eq(&self, other: &[T; N]) -> bool {
        self.0.as_slice() == other.as_slice()
    }
}

impl<T: Eq> Eq for Shared<T> {}

impl<T: std::fmt::Debug> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> From<Vec<T>> for Shared<T> {
    fn from(v: Vec<T>) -> Self {
        Self::new(v)
    }
}

impl<T> FromIterator<T> for Shared<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl<'a, T> IntoIterator for &'a Shared<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T: Serialize> Serialize for Shared<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Shared<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Vec::deserialize(d).map(Self::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_until_written_and_compare_by_value() {
        let a: Shared<u16> = vec![1, 2, 3].into();
        let mut b = a.clone();
        assert!(a.ptr_eq(&b) && a == b);
        b[1] = 9;
        assert!(!a.ptr_eq(&b));
        assert_eq!((a[1], b[1]), (2, 9), "the write copied, the original is unchanged");
        let c: Shared<u16> = [1u16, 2, 3].into_iter().collect();
        assert!(!a.ptr_eq(&c) && a == c, "separate buffers still compare by contents");
        assert_eq!(a, vec![1, 2, 3]);
        assert_eq!(serde_json::to_string(&a).unwrap(), "[1,2,3]");
        assert_eq!(serde_json::from_str::<Shared<u16>>("[1,2,3]").unwrap(), a);
        assert_eq!(b.into_vec(), vec![1, 9, 3]);
    }
}
