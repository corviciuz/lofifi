use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn hash_string(input: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    hasher.finish()
}

pub trait HasId {
    fn get_id(&self) -> Option<u64>;
}

impl HasId for crate::bandcamp::DiscographyItem {
    fn get_id(&self) -> Option<u64> {
        self.id
    }
}

pub fn hash_items_with_ids<T: HasId>(items: &[T]) -> u64 {
    let mut hasher = DefaultHasher::new();

    let mut ids: Vec<u64> = items.iter()
        .filter_map(|item| item.get_id())
        .collect();
    ids.sort();

    for id in ids {
        id.hash(&mut hasher);
    }
    hasher.finish()
}
