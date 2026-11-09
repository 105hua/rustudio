//! Studio budget pools: getter assembly, observed GetFVariable
//! defaults and paired legacy/modern consumption probes in checks/parity/data.md.
use crate::runtime::World;
use mlua::{Lua, MultiValue, Value};
const BASE: [f32; 17] = [
    60., 60., 0., 5., 30., 0., 5., 5., 5., 60., 60., 5., 60., 60., 30., 5., 30.,
];
const PER: [f32; 17] = [
    40., 40., 0., 2., 5., 0., 2., 2., 2., 40., 40., 2., 40., 40., 5., 2., 5.,
];
const INITIAL: [f32; 17] = [
    100., 100., 0., 10., 50., 0., 10., 10., 10., 100., 100., 10., 100., 100., 50., 10., 50.,
];
#[derive(Clone)]
pub struct Budget {
    tokens: [f32; 17],
    base: [f32; 17],
    per: [f32; 17],
    configured: [bool; 17],
    tick: u64,
    queued: [u32; 17],
    watched_keys: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            tokens: INITIAL,
            base: BASE,
            per: PER,
            configured: [false; 17],
            tick: 0,
            queued: [0; 17],
            watched_keys: 0,
        }
    }
}
fn error(s: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(s.into())
}
fn context(lua: &Lua) -> (f64, usize) {
    let Some(w) = lua.app_data_ref::<World>() else {
        return (0., 0);
    };
    let d = w.d();
    let n = d
        .find_child(d.dom.root_ref(), "Players")
        .and_then(|r| d.dom.get_by_ref(r))
        .map(|i| {
            i.children()
                .iter()
                .filter(|r| d.class_of(**r).is_some_and(|s| s == "Player"))
                .count()
        })
        .unwrap_or(0);
    (w.time, n)
}
impl Budget {
    fn refresh(&mut self, time: f64, players: usize) {
        let tick = time.max(0.).floor() as u64;
        let ticks = tick.saturating_sub(self.tick);
        for _ in 0..ticks.min(1_000_000) {
            for i in 0..17 {
                if matches!(i, 2 | 5) {
                    continue;
                }
                let rate = self.base[i] + self.per[i] * players as f32;
                self.tokens[i] = (self.tokens[i] + rate / 60.).min(rate * 100.).max(0.);
            }
        }
        self.tick = tick;
    }
    fn get(&self, index: usize, players: usize) -> i32 {
        match index {
            2 => self.tokens[0].min(self.tokens[1]).max(0.) as i32,
            5 => (((30 + 5 * players) * 30 / 60) as i32 - self.watched_keys as i32).max(0),
            _ => self.tokens[index].max(0.) as i32,
        }
    }
}
pub fn call(lua: &Lua, name: &str, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if !matches!(
        name,
        "GetRequestBudgetForRequestType" | "SetRateLimitForRequestType"
    ) {
        return None;
    }
    Some((|| {
        let v = args
            .get(1)
            .filter(|v| !matches!(v, Value::Nil))
            .ok_or_else(|| error("Argument 1 missing or nil"))?;
        let item = crate::ltypes::enum_arg(Some(v), "DataStoreRequestType", "GetAsync")?;
        let i = item.value as usize;
        if i > 16 {
            return Err(error(
                "GetRequestBudgetForRequestType: Unknown request type.",
            ));
        }
        let (time, players) = context(lua);
        let mut b = lua.app_data_mut::<Budget>().unwrap();
        b.refresh(time, players);
        if name == "GetRequestBudgetForRequestType" {
            return Ok(MultiValue::from_vec(vec![Value::Integer(
                b.get(i, players) as i64,
            )]));
        }
        let number = |index: usize| -> mlua::Result<f32> {
            match args.get(index) {
                Some(Value::Number(n)) => Ok(*n as f32),
                Some(Value::Integer(n)) => Ok(*n as f32),
                _ => Err(error(format!("Argument {index} missing or nil"))),
            }
        };
        let base = number(2)?;
        let per = number(3)?;
        if matches!(i, 2 | 5) {
            return Err(error(format!(
                "SetRateLimitForRequestType: Does not apply to DataStoreRequestType.{}.",
                item.name
            )));
        }
        let max_base = if i >= 9 { 10000. } else { BASE[i] };
        let max_per = if i >= 9 { 200. } else { PER[i] };
        for (label, value, max) in [("base", base, max_base), ("per-player", per, max_per)] {
            if !value.is_finite() || value < 0. || value > max {
                return Err(error(format!("SetRateLimitForRequestType: proposed {label} limit {value} for DataStoreRequestType.{} is out of range [0, {max}].",item.name)));
            }
        }
        let repeat = b.configured[i];
        b.configured[i] = true;
        b.base[i] = base;
        b.per[i] = per;
        drop(b);
        if repeat {
            if let Some(mut w) = lua.app_data_mut::<World>() {
                w.output.push((crate::app::Level::Warn,format!("SetRateLimitForRequestType: Rate limit for DataStoreRequestType.{} has already been set. Please ensure this is intended behavior.",item.name)))
            }
        }
        Ok(MultiValue::new())
    })())
}
fn cost(method: &str, path: &str) -> (usize, usize) {
    if path.starts_with("/v1/") {
        if path.contains("/list?") {
            (3, 15)
        } else if method == "GET" || path.ends_with("/multi-get") {
            (0, 13)
        } else if path.contains("/remove?") {
            (4, 16)
        } else {
            (4, 14)
        }
    } else if method == "DELETE" {
        if path.contains("&version=") {
            (8, 12)
        } else {
            (1, 12)
        }
    } else if method == "POST" {
        (1, 10)
    } else if path.contains("/versions?")
        || path.contains("/objects?")
        || path.contains("/datastores?")
    {
        (6, 11)
    } else if path.contains("&version=") || path.contains("&versionCurrentTime=") {
        (7, 9)
    } else {
        (0, 9)
    }
}
pub fn watch_key(lua: &Lua) -> mlua::Result<()> {
    lua.app_data_mut::<Budget>().unwrap().watched_keys += 1;
    Ok(())
}
pub fn consume(lua: &Lua, method: &str, path: &str, units: usize) -> mlua::Result<bool> {
    let (time, players) = context(lua);
    let mut b = lua.app_data_mut::<Budget>().unwrap();
    b.refresh(time, players);
    let (a, c) = cost(method, path);
    let units = units as f32;
    if b.tokens[a] < units || b.tokens[c] < units {
        return Ok(false);
    }
    b.tokens[a] -= units;
    b.tokens[c] -= units;
    Ok(true)
}
pub fn queue(lua: &Lua, method: &str, path: &str, delta: i32, operation: &str) -> mlua::Result<()> {
    let (a, _) = cost(method, path);
    let mut b = lua.app_data_mut::<Budget>().unwrap();
    if delta < 0 {
        b.queued[a] = b.queued[a].saturating_sub(1);
        return Ok(());
    }
    if b.queued[a] >= 30 {
        let (code, label) = match operation {
            "SetAsync" => (302, "SetAsync"),
            "IncrementAsync" => (303, "IncrementAsync"),
            "UpdateAsync" => (304, "UpdateAsync"),
            "RemoveAsync" => (306, "RemoveAsync"),
            "GetSorted" => (305, "GetSorted"),
            _ => (301, "GetAsync"),
        };
        return Err(error(format!("{code}: {label} request dropped. Request was throttled, but throttled request queue was full.")));
    }
    b.queued[a] += 1;
    drop(b);
    if let Some(mut w) = lua.app_data_mut::<World>() {
        w.output.push((crate::app::Level::Warn,"DataStore request was added to queue. If request queue fills, further requests will be dropped. Try sending fewer requests.".into()));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn studio_initial_refill_caps_and_update_min() {
        let mut b = Budget::default();
        assert_eq!(b.get(0, 0), 100);
        assert_eq!(b.get(3, 0), 10);
        assert_eq!(b.get(4, 0), 50);
        assert_eq!(b.get(5, 0), 15);
        b.refresh(1., 0);
        assert_eq!(b.get(0, 0), 101);
        b.tokens[1] = 20.;
        assert_eq!(b.get(2, 0), 20);
        b.refresh(100000., 0);
        assert_eq!(b.get(0, 0), 6000);
        assert_eq!(b.get(3, 0), 500);
        assert_eq!(b.get(4, 0), 3000);
        b.base[0] = 0.;
        b.per[0] = 0.;
        assert_eq!(b.get(0, 0), 6000);
        b.refresh(100001., 0);
        assert_eq!(b.get(0, 0), 0);
    }
}
