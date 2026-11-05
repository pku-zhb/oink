//! Development-only fixture. Its transport never connects to a native backend.
import { OinkProvider } from 'oink-ui';
import { agent } from '../api/store';
import type { AgentState } from '../api/store';
import { transport } from '../api/transport';
import type { DesktopRequest, DesktopState, Inspection, Panel } from '../api/physicalDesktop';
import { setLanguage } from '../i18n';
import { PhysicalMonitors } from './PhysicalMonitors';

const panels: Panel[] = ['Left monitor','Right monitor'].map((name,i)=>({id:`panel-${i}`,name,width:1,height:1,baseline:i?'windows':'mac',readback:i===0,calibrated:true,
  endpoints:['mac','windows'].map((device,j)=>({screen:{device,display:`${device}-${i}`},input:17+j,port:`HDMI ${j+1}`}))}));
let fixture: DesktopState={mapping:{revision:1,group:'preview',coordinator:'mac',panels},imported:null,active:[panels[0].endpoints[0].screen,panels[1].endpoints[1].screen],mode:'compatible',operation:null,has_entry:true,group:'preview',cooldown_until:0,recovery:null};
setLanguage(new URLSearchParams(location.search).get('language')??'zh-Hans');
agent.adopt('overview',{this_device:{id:'mac',name:'Mac'}} as AgentState['overview']);
agent.adopt('cards',{cards:[{device:'mac',name:'Mac'},{device:'windows',name:'Windows'}]} as AgentState['cards']);
transport.call=async <R,>(request:{method:string;params?:Record<string,unknown>}):Promise<R>=>{
  if(request.method==='layout')return {revision:1,source:'manual',geometry:{screens:[]}} as R;
  const command=(request.params?.command as {request:DesktopRequest}).request;
  if(command.action==='inspect')return {device:command.device,desktop_supported:true,ddc_supported:true,desktop_verified:false,
    displays:panels.map((p,i)=>({key:`${command.device}-${i}`,name:p.name,bounds:{x:i*1920,y:0,w:1920,h:1080},pixel_width:1920,pixel_height:1080,rotation_deg:0,is_primary:i===0,is_builtin:false,is_virtual:false}))} satisfies Inspection as R;
  if(command.action==='read')return {input:command.device==='mac'?17:18} as R;
  if(command.action==='apply'){
    const operation={id:crypto.randomUUID(),phase:'awaiting_confirmation',detail:'',deadline_ms:Date.now()+15000,choices:command.choices,mode:command.mode,hardware_verified:false};
    fixture={...fixture,operation};window.setTimeout(()=>{if(fixture.operation===operation)fixture={...fixture,operation:{...operation,phase:'reverted',deadline_ms:null}};},15000);
  }
  if(command.action==='keep'&&fixture.operation)fixture={...fixture,active:fixture.operation.choices,has_entry:true,operation:{...fixture.operation,phase:'completed',deadline_ms:null}};
  if(command.action==='revert'&&fixture.operation)fixture={...fixture,operation:{...fixture.operation,phase:'reverted',deadline_ms:null}};
  if(command.action==='restore'||command.action==='recover_local')fixture={...fixture,active:[],has_entry:false,operation:null};
  if(command.action==='save')fixture={...fixture,mapping:{...command.mapping,revision:1}};
  if(command.action==='calibrate')fixture={...fixture,operation:{id:crypto.randomUUID(),phase:'calibration_ready',detail:'',deadline_ms:null,choices:[],mode:null,hardware_verified:false}};
  return structuredClone(fixture) as R;
};
export function PhysicalPreview(){return <OinkProvider defaultMotion="reduced"><main style={{maxWidth:760,margin:'40px auto',padding:24}}><p style={{opacity:.55,fontSize:12}}>Development preview · simulated displays only</p><PhysicalMonitors/></main></OinkProvider>;}
