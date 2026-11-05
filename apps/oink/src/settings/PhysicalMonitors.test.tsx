// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, expect, it, vi } from 'vitest';
import { OinkProvider } from 'oink-ui';
import { setLanguage } from '../i18n';
import { PhysicalMonitors } from './PhysicalMonitors';
const mocks = vi.hoisted(() => ({ call:vi.fn(), query:vi.fn(), state:{} as Record<string,unknown> }));
vi.mock('../api/store', () => ({useAgent:()=>mocks.state,agent:{query:mocks.query}}));
vi.mock('../api/client', () => ({call:mocks.call,api:{layout:vi.fn(),identifyScreens:vi.fn()}}));
const key=(device:string,display:string)=>({device,display});
const setup={revision:2,monitors:[
  {name:'Left',controller_display:'ml',bindings:[{screen:key('mac','ml'),input:17},{screen:key('win','wl'),input:15}]},
  {name:'Right',controller_display:'mr',bindings:[{screen:key('mac','mr'),input:17},{screen:key('win','wr'),input:8}]},
]};
beforeAll(()=>{
  Object.defineProperty(window,'matchMedia',{value:()=>({matches:true,addEventListener(){},removeEventListener(){}})});
  vi.stubGlobal('ResizeObserver',class{observe(){} unobserve(){} disconnect(){}});
});
beforeEach(()=>{
  vi.clearAllMocks(); setLanguage('en');
  mocks.state={overview:{this_device:{id:'mac'}},layout:{geometry:{screens:[['mac','ml'],['mac','mr'],['win','wl'],['win','wr']].map(([device,display])=>({key:key(device,display),device_name:device==='mac'?'Mac':'Win',display:{name:display,is_virtual:false}}))}}};
  mocks.call.mockResolvedValue({setup,active:[],controller_available:true});
  mocks.query.mockResolvedValue(undefined);
});
afterEach(cleanup);
const view=()=> <OinkProvider defaultMotion="reduced"><PhysicalMonitors/></OinkProvider>;
it('loads without changing inputs, and exposes all four physical ownership combinations',async()=>{
  const result=render(view());
  for(const label of ['Mac / Mac','Mac / Win','Win / Mac','Win / Win']) await result.findByRole('button',{name:label});
  expect(mocks.call).toHaveBeenCalledExactlyOnceWith('physical_monitors',{command:{action:'query'}});
});
it('switches the physical left and right screens in the selected order',async()=>{
  const result=render(view());
  fireEvent.click(await result.findByRole('button',{name:'Win / Mac'}));
  await waitFor(()=>expect(mocks.call).toHaveBeenCalledWith('physical_monitors',{command:{action:'apply',expected_revision:2,screens:[key('win','wl'),key('mac','mr')]}}));
  await result.findByText('Monitor inputs confirmed; physical layout applied');
  expect(mocks.query).toHaveBeenCalledWith('layout',expect.any(Function));
});
it('reports a failed switch without claiming the scene was applied',async()=>{
  mocks.call.mockImplementation((_method,params)=> params.command.action==='apply' ? Promise.resolve({error:'Input restoration needs attention'}) : Promise.resolve({setup,active:[],controller_available:true}));
  const result=render(view());
  fireEvent.click(await result.findByRole('button',{name:'Mac / Win'}));
  expect((await result.findByRole('alert')).textContent).toContain('Input restoration needs attention');
  expect(result.queryByText('Monitor inputs confirmed; physical layout applied')).toBeNull();
});
it('returns to ordinary routing without sending an input-source command',async()=>{
  mocks.call.mockResolvedValueOnce({setup,active:[key('mac','ml'),key('win','wr')],controller_available:true});
  const result=render(view());
  fireEvent.click(await result.findByRole('button',{name:'Return to ordinary layout'}));
  await waitFor(()=>expect(mocks.call).toHaveBeenCalledWith('physical_monitors',{command:{action:'clear'}}));
});
it('keeps computer names available when a scene hides all of one computer’s screens',async()=>{
  mocks.state={...mocks.state,
    cards:{cards:[{device:'mac',name:'Mac'},{device:'win',name:'Win'}]},
    layout:{geometry:{screens:[{key:key('win','wl'),device_name:'Win',display:{name:'Left',is_virtual:false}}]}},
  };
  const result=render(view());
  await result.findByRole('button',{name:'Mac / Mac'});
  await result.findByRole('button',{name:'Win / Mac'});
});
it('accepts a hexadecimal input value without discarding the typed prefix',async()=>{
  const result=render(view());
  const input=await result.findByRole('textbox',{name:'Left · Mac · Input value'});
  fireEvent.change(input,{target:{value:'0x'}});
  expect((input as HTMLInputElement).value).toBe('0x');
  fireEvent.change(input,{target:{value:'0x12'}});
  fireEvent.click(result.getByRole('button',{name:'Save bindings'}));
  await waitFor(()=>expect(mocks.call).toHaveBeenCalledWith('physical_monitors',{command:expect.objectContaining({action:'save',monitors:expect.arrayContaining([
    expect.objectContaining({name:'Left',bindings:expect.arrayContaining([{screen:key('mac','ml'),input:18}])}),
  ])})}));
});
it('uses the visually confirmed baseline to test a monitor without readback',async()=>{
  const calibrated={...setup,monitors:setup.monitors.map((m,i)=>i===0?{...m,readback:false,initial_input:17}:m)};
  mocks.call.mockResolvedValue({setup:calibrated,active:[],controller_available:true});
  const result=render(view());
  await result.findByText('This mode sends commands using visually confirmed mappings. Confirm the current picture before first use or after manual input changes; actual inputs cannot be verified.');
  expect((result.getAllByRole('button',{name:'Read current input'})[0] as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(result.getAllByRole('button',{name:'Test and restore'})[0]);
  await waitFor(()=>expect(mocks.call).toHaveBeenCalledWith('physical_monitors',{command:{action:'test',controller_display:'ml',input:17,restore_input:17}}));
  await result.findByText('Test and restore commands sent. Verify the picture visually; the monitor cannot confirm it by readback.');
});
it('requires a fresh picture confirmation before entering calibrated physical mode',async()=>{
  const calibrated={...setup,monitors:setup.monitors.map((m,i)=>i===0?{...m,readback:false,initial_input:17}:m)};
  mocks.call.mockResolvedValue({setup:calibrated,active:[],controller_available:true});
  const result=render(view());
  const scene=await result.findByRole('button',{name:'Mac / Win'});
  expect((scene as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(result.getByRole('button',{name:'Save bindings and confirm current picture'}));
  await waitFor(()=>expect((scene as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(scene);
  await waitFor(()=>expect(mocks.call).toHaveBeenCalledWith('physical_monitors',{command:{action:'apply',expected_revision:2,screens:[key('mac','ml'),key('win','wr')],confirmed_inputs:{ml:17}}}));
});
