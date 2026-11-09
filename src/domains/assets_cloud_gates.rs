//! Studio Play feature gates and script capability checks, verified in an
//! ordinary Script/LocalScript in the shipped Studio; see parity notes.
use super::*;
pub const LUA_METHODS:&[(&str,&str)]=&[
    ("CreateAssetAsync","AssetService"),("CreateAssetVersionAsync","AssetService"),("PromptImportAnimationClipFromVideoAsync","AssetService"),
    ("CreateSessionAsync","MLService"),("LoadPostProcessModelAsync","MLService"),
    ("SetPostProcessEnabled","MLService"),("IsPostProcessReady","MLService"),("ForwardAsync","MLSession"),
    ("GetSchema","FeatureMatchingTable"),
    ("SegmentMeshAsync","GenerationService"),("LoadGeneratedMeshAsync","GenerationService"),
    ("GetVideoGenSessionAsync","GenerationService"),("GetVideoGenTriggersAsync","GenerationService"),
    ("StartVideoGenSessionAsync","GenerationService"),("UpdateVideoGenSessionPromptAsync","GenerationService"),("UpdateVideoGenSessionTriggersAsync","GenerationService"),
    ("GetExecutingPlayer","RemoteCommandService"),("GetReceivedUpdateSignal","RemoteCommandService"),
    ("GetStoppingSignal","RemoteCommandService"),("SendUpdate","RemoteCommandService"),
    ("Call","AppHandle"),("Close","AppHandle"),("ExecuteLuau","AppHandle"),("RunTest","AppHandle"),
    ("Click","ParticipantHandle"),("InputText","ParticipantHandle"),("Launch","ParticipantHandle"),
    ("Screenshot","ParticipantHandle"),("Scroll","ParticipantHandle"),("Shutdown","ParticipantHandle"),
    ("GetChannels","FeatureMatchingSession"),
    ("GetDimensionCount","FeatureMatchingSession"),
    ("GetFeatureRow","FeatureMatchingSession"),
    ("GetFeatureRows","FeatureMatchingSession"),
    ("GetMetadataRow","FeatureMatchingSession"),
    ("GetRowCount","FeatureMatchingSession"),
    ("GetSchemaName","FeatureMatchingSession"),
    ("GetTagColumns","FeatureMatchingSession"),
    ("StandardizeQuery","FeatureMatchingSession"),
    ("GetDeviceInfo","CaptureService"),
    ("InternalCheckPlayabilityAsync","CaptureService"),
    ("InternalGetStartPlaceIdAsync","CaptureService"),
    ("Load","FeatureMatchingService"),
    ("LoadAsync","FeatureMatchingService"),
    ("ConnectAsync","GenerationService"),
    ("DisconnectAsync","GenerationService"),
    ("Disconnect","RealtimeMedia"),
    ("GetConnectedWires","RealtimeMedia"),
    ("GetInputPins","RealtimeMedia"),
    ("GetOutputPins","RealtimeMedia"),
    ("SendMessage","RealtimeMedia"),
    ("ConnectAsync","RealtimeMedia"),
    ("GetApiV1","OpenCloudService"),("InvokeAsync","OpenCloudService"),
];
pub fn install(lua:&Lua)->mlua::Result<()> {
    lua.globals().set("__ac_is_server",lua.create_function(|lua,()|Ok(world(lua)?.side != Side::Client))?)?;
    lua.load(r#"
local M=__rs_lua_methods
if Instance then
    local new=Instance.new
    Instance.new=function(class,...)
        if class=='RealtimeMedia' then error("The current thread cannot create 'RealtimeMedia' (lacking capability InternalTest)",0) end
        return new(class,...)
    end
end
local function disabled(message,yielding)
    return function(self,...)
        if yielding then task.wait() end
        error(message,0)
    end
end
-- Shared CreateAsset*/Version guard, , refuses a Play
-- datamodel before any upload (the edit-datamodel API is outside Play).
M.CreateAssetAsync=disabled('CreateAssetAsync can only be used in the Edit datamodel',true)
M.CreateAssetVersionAsync=M.CreateAssetAsync
-- Verified false in StudioAppSettings; the only place override is 14883099012.
M.PromptImportAnimationClipFromVideoAsync=disabled('AssetService::PromptImportAnimationClipFromVideoAsync feature not enabled.',true)
M.CreateSessionAsync=disabled('MLService is not enabled',true)
M.ForwardAsync=disabled('MLService is not enabled',true)
M.LoadPostProcessModelAsync=disabled('MLPostProcess is not enabled',true)
M.SetPostProcessEnabled=disabled('MLPostProcess is not enabled',false)
M.IsPostProcessReady=function() return false end
M.GetSchema=disabled('FeatureMatchingTable:GetSchema is not yet enabled',false)
M.SegmentMeshAsync=disabled('SegmentMeshAsync can only be called from Roblox Studio, not from a running experience.',true)
M.LoadGeneratedMeshAsync=function(self,id)
    if id==nil then error('Argument 1 missing or nil',2) end
    task.wait()
    if __ac_is_server() then error('loadGeneratedMeshAsync can currently only be called from the client',0) end
    error('Unable to replicate mesh',0)
end
for _,name in {'GetVideoGenSessionAsync','GetVideoGenTriggersAsync','StartVideoGenSessionAsync','UpdateVideoGenSessionPromptAsync','UpdateVideoGenSessionTriggersAsync'} do
    M[name]=disabled('VideoGen service is not enabled.',true)
end
for _,name in {'GetExecutingPlayer','GetReceivedUpdateSignal','GetStoppingSignal','SendUpdate'} do
    M[name]=disabled(name..' can only be called from within an executed command',false)
end
for _,name in {'Call','Close','ExecuteLuau','RunTest'} do
    M[name]=disabled('AppHandle:'..name..': owning TestCoordinatorService is no longer available',false)
end
for _,name in {'Click','InputText','Launch','Screenshot','Scroll','Shutdown'} do
    M[name]=disabled('ParticipantHandle:'..name..': owning TestCoordinatorService is no longer available',false)
end
-- Capability checks happen before entering the native method body.
M.Call=disabled("The current thread cannot call 'Call' (lacking capability InternalTest)",false)
M.Close=disabled("The current thread cannot call 'Close' (lacking capability InternalTest)",false)
M.ExecuteLuau=disabled("The current thread cannot call 'ExecuteLuau' (lacking capability InternalTest)",false)
M.RunTest=disabled("The current thread cannot call 'RunTest' (lacking capability InternalTest)",false)
M.GetChannels=disabled("The current thread cannot call 'GetChannels' (lacking capability InternalTest)",false)
M.GetDimensionCount=disabled("The current thread cannot call 'GetDimensionCount' (lacking capability InternalTest)",false)
M.GetFeatureRow=disabled("The current thread cannot call 'GetFeatureRow' (lacking capability InternalTest)",false)
M.GetFeatureRows=disabled("The current thread cannot call 'GetFeatureRows' (lacking capability InternalTest)",false)
M.GetMetadataRow=disabled("The current thread cannot call 'GetMetadataRow' (lacking capability InternalTest)",false)
M.GetRowCount=disabled("The current thread cannot call 'GetRowCount' (lacking capability InternalTest)",false)
M.GetSchemaName=disabled("The current thread cannot call 'GetSchemaName' (lacking capability InternalTest)",false)
M.GetTagColumns=disabled("The current thread cannot call 'GetTagColumns' (lacking capability InternalTest)",false)
M.StandardizeQuery=disabled("The current thread cannot call 'StandardizeQuery' (lacking capability InternalTest)",false)
M.GetDeviceInfo=disabled("The current thread cannot call 'GetDeviceInfo' (lacking capability InternalTest)",false)
M.InternalCheckPlayabilityAsync=disabled("The current thread cannot call 'InternalCheckPlayabilityAsync' (lacking capability InternalTest)",false)
M.InternalGetStartPlaceIdAsync=disabled("The current thread cannot call 'InternalGetStartPlaceIdAsync' (lacking capability InternalTest)",false)
M.Load=disabled("The current thread cannot call 'Load' (lacking capability InternalTest)",false)
M.LoadAsync=disabled("The current thread cannot call 'LoadAsync' (lacking capability InternalTest)",false)
M.ConnectAsync=disabled("The current thread cannot call 'ConnectAsync' (lacking capability InternalTest)",false)
M.DisconnectAsync=disabled("The current thread cannot call 'DisconnectAsync' (lacking capability InternalTest)",false)
M.GetVideoGenSessionAsync=disabled("The current thread cannot call 'GetVideoGenSessionAsync' (lacking capability InternalTest)",false)
M.GetVideoGenTriggersAsync=disabled("The current thread cannot call 'GetVideoGenTriggersAsync' (lacking capability InternalTest)",false)
M.StartVideoGenSessionAsync=disabled("The current thread cannot call 'StartVideoGenSessionAsync' (lacking capability InternalTest)",false)
M.UpdateVideoGenSessionPromptAsync=disabled("The current thread cannot call 'UpdateVideoGenSessionPromptAsync' (lacking capability InternalTest)",false)
M.UpdateVideoGenSessionTriggersAsync=disabled("The current thread cannot call 'UpdateVideoGenSessionTriggersAsync' (lacking capability InternalTest)",false)
M.IsPostProcessReady=disabled("The current thread cannot call 'IsPostProcessReady' (lacking capability InternalTest)",false)
M.SetPostProcessEnabled=disabled("The current thread cannot call 'SetPostProcessEnabled' (lacking capability InternalTest)",false)
M.LoadPostProcessModelAsync=disabled("The current thread cannot call 'LoadPostProcessModelAsync' (lacking capability InternalTest)",false)
M.Disconnect=disabled("The current thread cannot call 'Disconnect' (lacking capability InternalTest)",false)
M.GetConnectedWires=disabled("The current thread cannot call 'GetConnectedWires' (lacking capability InternalTest)",false)
M.GetInputPins=disabled("The current thread cannot call 'GetInputPins' (lacking capability InternalTest)",false)
M.GetOutputPins=disabled("The current thread cannot call 'GetOutputPins' (lacking capability InternalTest)",false)
M.SendMessage=disabled("The current thread cannot call 'SendMessage' (lacking capability InternalTest)",false)
M.ConnectAsync=disabled("The current thread cannot call 'ConnectAsync' (lacking capability InternalTest)",false)
M.GetExecutingPlayer=disabled("The current thread cannot call 'GetExecutingPlayer' (lacking capability RemoteCommand)",false)
M.GetReceivedUpdateSignal=disabled("The current thread cannot call 'GetReceivedUpdateSignal' (lacking capability RemoteCommand)",false)
M.GetStoppingSignal=disabled("The current thread cannot call 'GetStoppingSignal' (lacking capability RemoteCommand)",false)
M.SendUpdate=disabled("The current thread cannot call 'SendUpdate' (lacking capability RemoteCommand)",false)
M.Click=disabled("The current thread cannot call 'Click' (lacking capability InternalTest)",false)
M.InputText=disabled("The current thread cannot call 'InputText' (lacking capability InternalTest)",false)
M.Launch=disabled("The current thread cannot call 'Launch' (lacking capability InternalTest)",false)
M.Screenshot=disabled("The current thread cannot call 'Screenshot' (lacking capability InternalTest)",false)
M.Scroll=disabled("The current thread cannot call 'Scroll' (lacking capability InternalTest)",false)
M.Shutdown=disabled("The current thread cannot call 'Shutdown' (lacking capability InternalTest)",false)
M.GetApiV1=disabled('API is not enabled',false)
M.InvokeAsync=function(self,version,method,args,headers)
    if not __ac_is_server() then error('InvokeAsync should only be called from the server.',0) end
    if version==nil then error('Argument 1 missing or nil',2) end
    if method==nil then error('Argument 2 missing or nil',2) end
    error('Unable to find version.',0)
end

"#).set_name("=rustudio.assets_cloud.gates").exec()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_services_preserve_native_errors() {
        let lua=Lua::new();
        lua.globals().set("__rs_lua_methods",lua.create_table().unwrap()).unwrap();
        lua.load("task={wait=function() end}").exec().unwrap();
        install(&lua).unwrap();
        lua.load(r#"
local M=__rs_lua_methods
local ok,err=pcall(M.IsPostProcessReady)
assert(not ok and err=="The current thread cannot call 'IsPostProcessReady' (lacking capability InternalTest)")
for name,message in {CreateAssetAsync='CreateAssetAsync can only be used in the Edit datamodel',CreateAssetVersionAsync='CreateAssetAsync can only be used in the Edit datamodel',PromptImportAnimationClipFromVideoAsync='AssetService::PromptImportAnimationClipFromVideoAsync feature not enabled.',CreateSessionAsync='MLService is not enabled',LoadPostProcessModelAsync="The current thread cannot call 'LoadPostProcessModelAsync' (lacking capability InternalTest)",GetSchema='FeatureMatchingTable:GetSchema is not yet enabled',GetReceivedUpdateSignal="The current thread cannot call 'GetReceivedUpdateSignal' (lacking capability RemoteCommand)"} do
    local ok,err=pcall(M[name],{})
    assert(not ok and err==message, tostring(err))
end
"#).exec().unwrap();
    }
}
