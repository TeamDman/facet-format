//! Process-global cache for `TypePlanCore` built from typed `Facet` roots.
//!
//! The cache retains one `TypePlanCore` per distinct shape for the lifetime of
//! the process (no eviction). This trades bounded memory for fast shared plan
//! reuse in format-layer code.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use facet_core::Facet;
use facet_reflect::{AllocError, TypePlan, TypePlanCore};

fn cache() -> &'static Mutex<HashMap<&'static facet_core::Shape, Arc<TypePlanCore>>> {
    static PLAN_CACHE: OnceLock<Mutex<HashMap<&'static facet_core::Shape, Arc<TypePlanCore>>>> =
        OnceLock::new();
    PLAN_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Get a cached plan as `Arc<TypePlanCore>`, building on cache miss.
pub(crate) fn cached_type_plan_arc<'facet, T>() -> Result<Arc<TypePlanCore>, AllocError>
where
    T: Facet<'facet>,
{
    let mut guard = cache().lock().unwrap_or_else(|poison| poison.into_inner());

    if let Some(plan) = guard.get(&T::SHAPE) {
        return Ok(Arc::clone(plan));
    }

    let plan = TypePlan::<T>::build()?.core();
    guard.insert(T::SHAPE, Arc::clone(&plan));
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::{cache, cached_type_plan_arc};
    use facet::Facet;
    use facet_reflect::TypePlanCore;
    use std::sync::Arc;

    fn cache_entry<T: Facet<'static>>() -> Option<Arc<TypePlanCore>> {
        cache()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&T::SHAPE)
            .cloned()
    }

    #[test]
    fn cache_hit_miss_behavior() {
        #[derive(Facet)]
        #[allow(dead_code, reason = "fixture fields are inspected through Facet")]
        struct HitMissRoot(i32);

        assert!(cache_entry::<HitMissRoot>().is_none());

        let first = cached_type_plan_arc::<HitMissRoot>().unwrap();
        assert!(Arc::ptr_eq(&first, &cache_entry::<HitMissRoot>().unwrap()));

        let second = cached_type_plan_arc::<HitMissRoot>().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&second, &cache_entry::<HitMissRoot>().unwrap()));
    }

    #[test]
    fn cache_concurrent_access_single_shape() {
        #[derive(Facet)]
        #[allow(dead_code, reason = "fixture fields are inspected through Facet")]
        struct ConcurrentRoot(Option<Vec<u64>>);

        assert!(cache_entry::<ConcurrentRoot>().is_none());

        let mut joins = Vec::new();
        for _ in 0..12 {
            joins.push(std::thread::spawn(|| {
                let mut ptrs = Vec::new();
                for _ in 0..40 {
                    let plan = cached_type_plan_arc::<ConcurrentRoot>().unwrap();
                    ptrs.push(Arc::as_ptr(&plan) as usize);
                }
                ptrs
            }));
        }

        let mut all_ptrs = Vec::new();
        for join in joins {
            all_ptrs.extend(join.join().unwrap());
        }

        let first = *all_ptrs.first().unwrap();
        assert!(all_ptrs.into_iter().all(|ptr| ptr == first));
        assert_eq!(
            Arc::as_ptr(&cache_entry::<ConcurrentRoot>().unwrap()) as usize,
            first
        );
    }
}
