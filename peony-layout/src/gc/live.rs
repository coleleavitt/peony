use peony_object::InputObject;
use rustc_hash::FxHashSet;

pub struct LiveSections {
    words_by_object: Vec<Vec<u64>>,
    len: usize,
}

impl LiveSections {
    pub(super) fn new(objects: &[InputObject]) -> Self {
        let words_by_object = objects
            .iter()
            .map(|obj| {
                let max_index = obj
                    .sections
                    .iter()
                    .map(|sec| sec.index.0)
                    .max()
                    .unwrap_or(0);
                vec![0; (max_index / 64) + 1]
            })
            .collect();
        Self {
            words_by_object,
            len: 0,
        }
    }

    pub fn insert(&mut self, key: (usize, usize)) -> bool {
        let Some(words) = self.words_by_object.get_mut(key.0) else {
            return false;
        };
        let word_index = key.1 / 64;
        let Some(word) = words.get_mut(word_index) else {
            return false;
        };
        let mask = 1u64 << (key.1 % 64);
        if *word & mask != 0 {
            return false;
        }
        *word |= mask;
        self.len += 1;
        true
    }

    pub fn remove(&mut self, key: (usize, usize)) -> bool {
        let Some(words) = self.words_by_object.get_mut(key.0) else {
            return false;
        };
        let word_index = key.1 / 64;
        let Some(word) = words.get_mut(word_index) else {
            return false;
        };
        let mask = 1u64 << (key.1 % 64);
        if *word & mask == 0 {
            return false;
        }
        *word &= !mask;
        self.len -= 1;
        true
    }

    pub fn contains(&self, key: &(usize, usize)) -> bool {
        self.words_by_object
            .get(key.0)
            .and_then(|words| words.get(key.1 / 64))
            .is_some_and(|word| word & (1u64 << (key.1 % 64)) != 0)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn into_hash_set(self) -> FxHashSet<(usize, usize)> {
        let mut out = FxHashSet::default();
        out.reserve(self.len);
        for (object_id, words) in self.words_by_object.into_iter().enumerate() {
            for (word_index, mut word) in words.into_iter().enumerate() {
                while word != 0 {
                    let bit = usize::try_from(word.trailing_zeros()).unwrap_or(0);
                    out.insert((object_id, word_index * 64 + bit));
                    word &= word - 1;
                }
            }
        }
        out
    }
}
