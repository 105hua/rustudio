use super::*;
use rbx_dom_weak::InstanceBuilder;
#[test]
fn attachment_luau_frames_and_axes() {
    let mut d = Document::new_place();
    let sss = d.find_child(d.dom.root_ref(), "ServerScriptService").unwrap();
    let s = d.insert(sss, InstanceBuilder::new("Script").with_property("Source", r#"
        local p = Instance.new("Part", workspace)
        p.CFrame = CFrame.new(10,0,0) * CFrame.Angles(0,0,math.pi/2)
        local b = Instance.new("Bone", p)
        b.CFrame = CFrame.new(1,0,0)
        b.Transform = CFrame.new(0,1,0)
        local a = Instance.new("Attachment", b)
        a.CFrame = CFrame.new(1,0,0)
        assert((a.WorldPosition - Vector3.new(9,2,0)).Magnitude < .0001)
        assert((b.WorldPosition - Vector3.new(10,1,0)).Magnitude < .0001)
        assert((b.TransformedWorldCFrame.Position - Vector3.new(9,1,0)).Magnitude < .0001)
        assert((a.WorldAxis - Vector3.new(0,1,0)).Magnitude < .0001)
        assert((a.WorldSecondaryAxis - Vector3.new(-1,0,0)).Magnitude < .0001)
        assert(a:GetAxis() == Vector3.new(1,0,0))
        assert(a:GetSecondaryAxis() == Vector3.new(0,1,0))
        a.WorldPosition = Vector3.new(5,6,7)
        assert((a.WorldPosition - Vector3.new(5,6,7)).Magnitude < .0001)
        print("attachment frames passed")
    "#.to_string()));
    let rt = Runtime::new().unwrap();
    rt.enter(World::new(d, RunMode::Run));
    rt.bind_globals().unwrap();
    rt.run_script(s);
    for _ in 0..5 { rt.step(1.0/60.0); }
    let out = rt.leave().output;
    assert!(out.iter().all(|(level,_)| *level != Level::Error), "{out:?}");
    assert!(out.iter().any(|(_,s)| s.starts_with("attachment frames passed")), "{out:?}");
}
