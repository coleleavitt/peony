use std::borrow::Borrow;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Eq)]
pub struct Name(Arc<[u8]>);

impl Name {
    pub fn from_slice(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self::empty();
        }
        Self(Arc::from(bytes))
    }

    pub fn empty() -> Self {
        static EMPTY: OnceLock<Name> = OnceLock::new();
        EMPTY.get_or_init(|| Self(Arc::from([]))).clone()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl From<Vec<u8>> for Name {
    fn from(value: Vec<u8>) -> Self {
        if value.is_empty() {
            return Self::empty();
        }
        Self(Arc::from(value.into_boxed_slice()))
    }
}

impl From<&[u8]> for Name {
    fn from(value: &[u8]) -> Self {
        Self::from_slice(value)
    }
}

impl Deref for Name {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_bytes()
    }
}

impl Borrow<[u8]> for Name {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl PartialEq<[u8]> for Name {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_bytes() == other
    }
}

impl PartialEq<&[u8]> for Name {
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_bytes() == *other
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for Name {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self.as_bytes() == &other[..]
    }
}

impl std::fmt::Debug for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Name")
            .field(&String::from_utf8_lossy(self.as_bytes()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::Name;

    #[test]
    fn empty_names_share_backing_storage() {
        let first = Name::empty();
        let second = Name::from_slice(b"");
        let third = Name::from(Vec::new());

        assert!(first.ptr_eq(&second));
        assert!(second.ptr_eq(&third));
    }
}
