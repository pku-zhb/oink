// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, expect, it, vi } from 'vitest';
import { OinkProvider } from 'oink-ui';
import { setLanguage } from '../i18n';
import { PhysicalMonitors } from './PhysicalMonitors';
import { presets, suggestPanels } from '../api/physicalDesktop';
import type { DesktopState, Inspection, Panel } from '../api/physicalDesktop';

const mocks=vi.hoisted(()=>({call:vi.fn(),query:vi.fn(),agent:{} as Record<string,unknown>,reply:{} as Record<string,unknown>}));
vi.mock('../api/store',()=>({useAgent:()=>mocks.agent,agent:{query:mocks.query}}));
vi.mock('../api/client',()=>({call:mocks.call,api:{layout:vi.fn(),identifyScreens:vi.fn()}}));
const panels:Panel[]=['Left','Right'].map((name,i)=>({id:name,name,width:1,height:1,readback:true,baseline:'mac',calibrated:true,
  endpoints:['mac','win'].map((device,j)=>({screen:{device,display:`${device}-${i}`},input:17+j,port:j?'HDMI 2':'HDMI 1'}))}));
const initial=():DesktopState=>({mapping:{revision:1,group:'group',coordinator:'mac',panels},imported:null,active:[],mode:null,operation:null,has_entry:false,group:'group',cooldown_until:0,recovery:null});
beforeAll(()=>{
  Object.defineProperty(window,'matchMedia',{value:()=>({matches:true,addEventListener(){},removeEventListener(){}})});
  vi.stubGlobal('ResizeObserver',class{observe(){}unobserve(){}disconnect(){}});
});
beforeEach(()=>{
  vi.clearAllMocks();setLanguage('en');mocks.reply=initial() as unknown as Record<string,unknown>;
  mocks.agent={overview:{this_device:{id:'mac',name:'Mac'}},cards:{cards:[{device:'mac',name:'Mac'},{device:'win',name:'Windows'}]}};
  mocks.call.mockImplementation(async()=>mocks.reply);mocks.query.mockResolvedValue(undefined);
});
afterEach(cleanup);
const view=()=>render(<OinkProvider defaultMotion="reduced"><PhysicalMonitors/></OinkProvider>);
it('loads read-only and exposes the four visual monitor assignments',async()=>{
  const v=view();
  for(const name of ['Mac / Mac','Mac / Windows','Windows / Mac','Windows / Windows']) await v.findByRole('button',{name});
  expect(mocks.call.mock.calls.every(([,p])=>p.command.request.action==='status')).toBe(true);
});
it('applies a calibrated scene with an explicit compatible desktop policy',async()=>{
  const v=view();fireEvent.click(await v.findByRole('button',{name:'Windows / Mac'}));
  await waitFor(()=>expect(mocks.call).toHaveBeenCalledWith('physical_monitors',{command:{action:'experience',request:{action:'apply',expected_revision:1,choices:[panels[0].endpoints[1].screen,panels[1].endpoints[0].screen],mode:'compatible',baseline_confirmed:false}}}));
});
it('confirms the exact pending operation rather than a generic keep action',async()=>{
  mocks.reply={...initial(),operation:{id:'pending-7',phase:'awaiting_confirmation',detail:'',deadline_ms:Date.now()+15000,choices:[],mode:'complete',hardware_verified:false}};
  const v=view();fireEvent.click(await v.findByRole('button',{name:'Keep layout'}));
  await waitFor(()=>expect(mocks.call).toHaveBeenCalledWith('physical_monitors',{command:{action:'experience',request:{action:'keep',operation:'pending-7'}}}));
});
it('retains per-monitor controls when the Cartesian product exceeds the preset limit',async()=>{
  const many=Array.from({length:5},(_,i)=>({...panels[0],id:`p${i}`,name:`Monitor ${i}`}));
  mocks.reply={...initial(),mapping:{...initial().mapping!,panels:many}};
  expect(presets(many)).toEqual([]);
  const v=view();await v.findByRole('button',{name:'Apply layout'});
  for(const p of many) expect(await v.findByRole('combobox',{name:p.name})).toBeTruthy();
});
it('offers guided setup without running a monitor probe on entry',async()=>{
  mocks.reply={...initial(),mapping:null};const v=view();
  fireEvent.click(await v.findByRole('button',{name:'Set up shared monitors'}));
  await v.findByRole('button',{name:'Check and continue'});
  expect(mocks.call.mock.calls.every(([,p])=>p.command.request.action==='status')).toBe(true);
});
it('reports backend failures without claiming that a scene was applied',async()=>{
  mocks.call.mockImplementation(async(_method,p)=>p.command.request.action==='apply'?{error:'Native display configuration changed'}:mocks.reply);
  const v=view();fireEvent.click(await v.findByRole('button',{name:'Mac / Windows'}));
  expect((await v.findByRole('alert')).textContent).toContain('Native display configuration changed');
  expect(v.queryByText('Layout applied')).toBeNull();
});
it('does not auto-match indistinguishable monitors to one remote endpoint',()=>{
  const display={key:'local',name:'Same',bounds:{x:0,y:0,w:1920,h:1080},pixel_width:1920,pixel_height:1080,rotation_deg:0,is_primary:true,is_builtin:false,is_virtual:false};
  const local:Inspection={device:'mac',displays:[display],desktop_supported:true,ddc_supported:true,desktop_verified:false};
  const remote:Inspection={...local,device:'win',displays:[{...display,key:'one'},{...display,key:'two'}]};
  expect(suggestPanels(local,[remote])[0].endpoints).toHaveLength(1);
});
