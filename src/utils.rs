use std::collections::HashMap;
use std::hash::Hash;

pub trait GeneralMap<KeyType, ValueType>
where
    KeyType: Eq,
{
    fn new() -> Self;
    fn is_empty(&self) -> bool;
    fn len(&self) -> usize;
    fn any(&self, predicate: impl Fn(&ValueType) -> bool) -> bool;
    fn insert(&mut self, slightly_preferred_key: KeyType, value: ValueType) -> KeyType;
    fn find(
        &self,
        look_here_first: &[KeyType],
        predicate: impl Fn(&ValueType) -> bool,
    ) -> Option<KeyType>;
    fn remove(&mut self, key: &KeyType) -> Option<ValueType>;
    fn map_values<T>(&self, value_mapper: impl Fn(&ValueType) -> T) -> Vec<T>;
    fn find_all(&self, predicate: impl Fn(&ValueType) -> bool) -> Vec<KeyType>;
}

impl<KeyType, ValueType> GeneralMap<KeyType, ValueType> for HashMap<KeyType, ValueType>
where
    KeyType: Eq + Hash + Clone,
{
    fn new() -> Self {
        HashMap::new()
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn any(&self, predicate: impl Fn(&ValueType) -> bool) -> bool {
        self.values().any(predicate)
    }

    fn insert(&mut self, key: KeyType, value: ValueType) -> KeyType {
        let z = (self as &mut HashMap<_, _>).insert(key.clone(), value);
        if let Some(old_value) = z {
            (self as &mut HashMap<_, _>).remove(&key);
            (self as &mut HashMap<_, _>).insert(key.clone(), old_value);
            panic!("it was already present and we don't know where to put it instead");
        } else {
            key
        }
    }

    fn find(
        &self,
        look_here_first: &[KeyType],
        predicate: impl Fn(&ValueType) -> bool,
    ) -> Option<KeyType> {
        let in_look_here_first = look_here_first.iter().find_map(|key| {
            if let Some(value_found) = self.get(key) {
                if predicate(value_found) {
                    Some(key.clone())
                } else {
                    None
                }
            } else {
                None
            }
        });
        if let Some(found_early) = in_look_here_first {
            Some(found_early)
        } else {
            self.iter().find_map(|(key, value)| {
                if predicate(value) {
                    Some(key.clone())
                } else {
                    None
                }
            })
        }
    }

    fn remove(&mut self, key: &KeyType) -> Option<ValueType> {
        (self as &mut HashMap<_, _>).remove(key)
    }

    fn map_values<T>(&self, value_mapper: impl Fn(&ValueType) -> T) -> Vec<T> {
        self.values().map(value_mapper).collect()
    }

    fn find_all(&self, predicate: impl Fn(&ValueType) -> bool) -> Vec<KeyType> {
        self.iter()
            .filter_map(|(k, v)| if predicate(v) { Some(k.clone()) } else { None })
            .collect()
    }
}

impl<ValueType> GeneralMap<usize, ValueType> for Vec<Option<ValueType>> {
    fn new() -> Self {
        Vec::new()
    }

    fn is_empty(&self) -> bool {
        #[allow(clippy::redundant_closure_for_method_calls)]
        self.iter().all(|v| v.is_none())
    }

    fn len(&self) -> usize {
        self.iter()
            .fold(0, |acc, item| if item.is_none() { acc } else { acc + 1 })
    }

    fn any(&self, predicate: impl Fn(&ValueType) -> bool) -> bool {
        self.iter().any(|v| {
            if let Some(real_v) = v {
                predicate(real_v)
            } else {
                false
            }
        })
    }

    fn insert(&mut self, slightly_preferred_key: usize, value: ValueType) -> usize {
        let vec_len = (self as &Vec<_>).len();
        if slightly_preferred_key < vec_len && self[slightly_preferred_key].is_none() {
            self[slightly_preferred_key] = Some(value);
            slightly_preferred_key
        } else {
            for (where_to_put, cur_value) in self.iter_mut().enumerate() {
                if cur_value.is_none() {
                    *cur_value = Some(value);
                    return where_to_put;
                }
            }
            self.push(Some(value));
            vec_len
        }
    }

    fn find(
        &self,
        look_here_first: &[usize],
        predicate: impl Fn(&ValueType) -> bool,
    ) -> Option<usize> {
        let in_look_here_first = look_here_first.iter().find_map(|key| {
            if let Some(value_found) = &self[*key] {
                if predicate(value_found) {
                    Some(*key)
                } else {
                    None
                }
            } else {
                None
            }
        });
        if let Some(found_early) = in_look_here_first {
            Some(found_early)
        } else {
            self.iter().enumerate().find_map(|(idx, value)| {
                if let Some(real_value) = value {
                    if predicate(real_value) {
                        Some(idx)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        }
    }

    fn remove(&mut self, key: &usize) -> Option<ValueType> {
        let mut to_return = None;
        core::mem::swap(&mut to_return, &mut self[*key]);
        to_return
    }

    fn map_values<T>(&self, value_mapper: impl Fn(&ValueType) -> T) -> Vec<T> {
        self.iter()
            .filter_map(|value| {
                #[allow(clippy::redundant_closure)]
                value.as_ref().map(|v| value_mapper(v))
            })
            .collect()
    }

    fn find_all(&self, predicate: impl Fn(&ValueType) -> bool) -> Vec<usize> {
        self.iter()
            .enumerate()
            .filter_map(|(k, v)| {
                if let Some(real_v) = v {
                    if predicate(real_v) {
                        Some(k)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }
}
