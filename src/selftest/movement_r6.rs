//! Deterministic controller traces; no renderer, wall clock, assets or account.
use std::{collections::HashSet, path::Path};
use glam::Vec3;
use crate::{doc::{Document, new_part}, physics::{Physics, CharBody, HumShape}};

pub(super) fn run(dir: &Path) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut summary = Vec::new();
    for delay in [-6i32, -5, -4, -3, -2, -1, 0, 2, 4, 6, 8] {
        let ledge = delay >= 0;
        let name = if ledge { format!("ledge-{delay}") } else if delay == -6 { "standing".into() } else if delay == -5 { "jump-height".into() } else if delay == -1 { "jump".into() } else if delay == -3 { "jump-raw50".into() } else if delay == -4 { "air-control".into() } else { "walk-stop".into() };
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let children: Vec<_> = doc.dom.get_by_ref(ws).unwrap().children().to_vec();
        for r in children { doc.dom.destroy(r); }
        doc.dom.insert(ws, new_part("Floor", 1, Vec3::new(if ledge {-10.0} else {0.0}, -1.0, 0.0), Vec3::new(if ledge {20.0} else {400.0}, 2.0, 400.0), [128,128,128]).with_property("Anchored", true));
        let mut p = Physics::new(&mut doc); p.sync(&mut doc);
        let sh = HumShape { hip: 2.0, root_size: Vec3::new(2.0,2.0,1.0), box_lo: Vec3::new(-1.0,-1.0,-0.5), box_hi: Vec3::new(1.0,2.0,0.5), root_lift: 0.0, mass: 12.6, parts: 7, r6: true };
        let own = HashSet::new(); let mut yaw = 0.0;
        let mut c = CharBody { pos: Vec3::new(-4.0,3.0,0.0), ..Default::default() };
        for _ in 0..60 { p.move_humanoid(&doc,&mut c,&mut yaw,&sh,Vec3::ZERO,None,true,1.0/60.0,&own); }
        if delay == -4 { c.pos.y = 1000.0; c.vel = Vec3::ZERO; c.had_floor = false; c.grounded = false; }
        let base = c.pos.y; let mut peak = base; let mut left = None; let mut jumped = false;
        let mut state = "Running"; let mut landed = 0;
        let mut csv = String::from("frame,x,y,z,vx,vy,vz,state,grounded,jump_requested,jumped\n");
        for frame in 0..180 {
            // Delay is measured from the first frame whose BEGINNING is beyond
            // the geometric platform edge; floor-ray grace is measured separately.
            if ledge && c.pos.x > 0.0 && left.is_none() { left = Some(frame); }
            let request = if ledge { left.is_some_and(|f| frame == f + delay) } else { matches!(delay, -1 | -3 | -5) && frame == 0 };
            let walk = if ledge || matches!(delay, -2 | -4) && frame < 60 { Vec3::X*16.0 } else { Vec3::ZERO };
            let m = p.move_humanoid(&doc,&mut c,&mut yaw,&sh,walk,request.then_some(if delay == -5 {(2.0 * 196.2_f32 * 7.2).sqrt()} else if delay == -3 {50.0} else {53.0}),true,1.0/60.0,&own);
            // Same state ordering as play.rs; this is controller-derived state,
            // not a claim of observing Lua Humanoid events.
            state = if !m.grounded { if m.jumped || m.jumping {"Jumping"} else {"Freefall"} } else if matches!(state,"Freefall"|"Jumping") { landed=0; "Landed" } else if state=="Landed" && landed<3 { landed+=1; "Landed" } else {"Running"};
            if delay == -4 && frame == 0 && (c.vel.x - 143.0/60.0).abs() > 0.001 { return Err(format!("air acceleration {}",c.vel.x)); }
            if delay == -4 && frame == 80 && c.vel.x.abs() > 0.001 { return Err(format!("air braking {}",c.vel.x)); }
            peak=peak.max(c.pos.y); jumped |= m.jumped;
            csv.push_str(&format!("{frame},{},{},{},{},{},{},{state},{},{request},{}\n", c.pos.x,c.pos.y,c.pos.z,c.vel.x,c.vel.y,c.vel.z,m.grounded,m.jumped));
            if delay == -6 && ((c.pos.y-base).abs() > 0.001 || c.vel.y.abs() > 0.005) { return Err(format!("standing bob at {frame}: {} {}",c.pos.y-base,c.vel.y)); }
            if !c.pos.is_finite() { return Err(format!("{name}: nonfinite")); }
        }
        std::fs::write(dir.join(format!("{name}.csv")),csv).map_err(|e|e.to_string())?;
        if matches!(delay, -1 | -3 | -5) && (((peak-base) - if delay == -5 {7.31} else if delay == -3 {6.47} else {7.27}).abs() > 0.06 || !jumped) { return Err(format!("jump apex {}",peak-base)); }
        if delay == -2 && c.vel.x.abs()>0.001 {return Err("did not stop".into());}
        summary.push(format!("{name}: rise={:.6}, jumped={jumped}, edge_frame={left:?}",peak-base));
    }
    let text=summary.join("\n"); std::fs::write(dir.join("summary.txt"),&text).map_err(|e|e.to_string())?; Ok(text)
}

#[cfg(test)]
mod tests {
    #[test]
    fn deterministic_r6_traces() {
        let dir = std::env::var_os("RUSTUDIO_MOVEMENT_TRACE_DIR").map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("rustudio-movement-r6"));
        println!("{}", super::run(&dir).expect("R6 movement traces"));
    }
}
