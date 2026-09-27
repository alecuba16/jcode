use std::time::Duration;

use super::helpers::backdated_now;
use crate::tui::info_widget::MemoryInfo;

pub(super) fn gather_memory_info(
    memory_enabled: bool,
    working_dir: Option<String>,
) -> Option<MemoryInfo> {
    use std::sync::Mutex;
    use std::time::Instant;

    static CACHE: Mutex<Option<(Instant, Option<MemoryInfo>, bool)>> = Mutex::new(None);
    const TTL: Duration = Duration::from_secs(2);

    // When memory is disabled we still surface the stored counts (with a
    // DISABLED badge) so the user can see they have memories but recall is off.
    // Live activity and the sidecar model are suppressed in that case.
    let activity = if memory_enabled {
        crate::memory::get_activity()
    } else {
        None
    };
    let sidecar_model = if memory_enabled && crate::memory::memory_sidecar_enabled() {
        let sidecar = crate::sidecar::Sidecar::new();
        Some(format!(
            "{} · {}",
            sidecar.backend_name(),
            sidecar.model_name()
        ))
    } else {
        None
    };

    let finalize = |mut info: MemoryInfo| {
        info.activity = activity.clone();
        info.sidecar_model = sidecar_model.clone();
        info.disabled = !memory_enabled;
        info
    };

    if let Ok(mut guard) = CACHE.lock() {
        if let Some((ts, cached, refreshing)) = guard.as_mut() {
            if ts.elapsed() < TTL || *refreshing {
                return match cached.clone() {
                    Some(info) => Some(finalize(info)),
                    None => fallback_memory_info(memory_enabled, &activity, &sidecar_model),
                };
            }
            let stale = match cached.clone() {
                Some(info) => Some(finalize(info)),
                None => fallback_memory_info(memory_enabled, &activity, &sidecar_model),
            };
            *refreshing = true;
            let working_dir = working_dir.clone();
            std::thread::spawn(move || {
                let result = gather_memory_info_inner(working_dir);
                if let Ok(mut guard) = CACHE.lock() {
                    *guard = Some((Instant::now(), result, false));
                }
            });
            return stale;
        }

        *guard = Some((backdated_now(TTL + Duration::from_secs(1)), None, true));
        std::thread::spawn(move || {
            let result = gather_memory_info_inner(working_dir);
            if let Ok(mut guard) = CACHE.lock() {
                *guard = Some((Instant::now(), result, false));
            }
        });
    }

    fallback_memory_info(memory_enabled, &activity, &sidecar_model)
}

fn fallback_memory_info(
    memory_enabled: bool,
    activity: &Option<crate::memory_types::MemoryActivity>,
    sidecar_model: &Option<String>,
) -> Option<MemoryInfo> {
    // Always return Some when memory is enabled so the info panel shows
    // the memory line even with 0 memories.
    if !memory_enabled && activity.is_none() && sidecar_model.is_none() {
        return None;
    }
    Some(MemoryInfo {
        sidecar_available: crate::memory::memory_sidecar_enabled(),
        sidecar_model: sidecar_model.clone(),
        activity: activity.clone(),
        disabled: !memory_enabled,
        ..Default::default()
    })
}

fn gather_memory_info_inner(working_dir: Option<String>) -> Option<MemoryInfo> {
    let activity = crate::memory::get_activity();
    let sidecar_model = if crate::memory::memory_sidecar_enabled() {
        let sidecar = crate::sidecar::Sidecar::new();
        Some(format!(
            "{} · {}",
            sidecar.backend_name(),
            sidecar.model_name()
        ))
    } else {
        None
    };

    use crate::memory::MemoryManager;

    // Scope the manager to the session working dir so the project count reads
    // the same projects/<hash>.json store the memory tool writes (issue #491).
    let manager = match working_dir.as_deref() {
        Some(dir) if !dir.trim().is_empty() => MemoryManager::new().with_project_dir(dir),
        _ => MemoryManager::new(),
    };
    // Load errors fall back to an empty info block: display-only counts.
    let project_graph = match manager.load_project_graph() {
        Ok(graph) => Some(graph),
        Err(_) => None,
    };
    let global_graph = match manager.load_global_graph() {
        Ok(graph) => Some(graph),
        Err(_) => None,
    };

    let (project_count, global_count, by_category) = {
        let mut by_category = std::collections::HashMap::new();
        let project_count = project_graph
            .as_ref()
            .map(|p| {
                for entry in p.memories.values() {
                    *by_category.entry(entry.category.to_string()).or_insert(0) += 1;
                }
                p.memory_count()
            })
            .unwrap_or(0);
        let global_count = global_graph
            .as_ref()
            .map(|g| {
                for entry in g.memories.values() {
                    *by_category.entry(entry.category.to_string()).or_insert(0) += 1;
                }
                g.memory_count()
            })
            .unwrap_or(0);
        (project_count, global_count, by_category)
    };

    let total_count = project_count + global_count;
    let (graph_nodes, graph_edges) = crate::tui::info_widget::build_graph_topology(
        project_graph.as_ref(),
        global_graph.as_ref(),
    );

    if total_count > 0 || activity.is_some() || sidecar_model.is_some() {
        Some(MemoryInfo {
            total_count,
            project_count,
            global_count,
            by_category,
            sidecar_available: crate::memory::memory_sidecar_enabled(),
            sidecar_model,
            activity,
            disabled: false,
            graph_nodes,
            graph_edges,
        })
    } else {
        // Still return Some with 0 counts so the info panel shows
        // the memory line even when there are no memories yet.
        Some(MemoryInfo {
            total_count: 0,
            project_count: 0,
            global_count: 0,
            by_category,
            sidecar_available: crate::memory::memory_sidecar_enabled(),
            sidecar_model,
            activity,
            disabled: false,
            graph_nodes,
            graph_edges,
        })
    }
}
