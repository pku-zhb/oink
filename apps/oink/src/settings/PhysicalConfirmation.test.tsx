// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, expect, it, vi } from 'vitest';
import { OinkProvider } from 'oink-ui';
import { setLanguage } from '../i18n';
import { PhysicalConfirmation } from './PhysicalConfirmation';
import type { Confirmation } from '../api/physicalDesktop';

const mocks=vi.hoisted(()=>({call:vi.fn(),event:vi.fn(),frame:null as unknown}));
vi.mock('../api/store',()=>({agent:{onEvent:mocks.event}}));
vi.mock('../api/client',()=>({call:mocks.call}));
vi.mock('../api/transport',()=>({inTauri:false}));
beforeAll(()=>{
  Object.defineProperty(window,'matchMedia',{value:()=>({matches:true,addEventListener(){},removeEventListener(){}})});
  vi.stubGlobal('ResizeObserver',class{observe(){}unobserve(){}disconnect(){}});
});
beforeEach(()=>{
  vi.clearAllMocks();setLanguage('en');mocks.frame=null;mocks.event.mockReturnValue(()=>{});
  mocks.call.mockImplementation(async(_method,p)=>{
    const request=p.command.request;
    if(request.action==='confirmation')return mocks.frame;
    if(request.action==='keep')mocks.frame={...(mocks.frame as Confirmation),phase:'kept'};
    if(request.action==='revert')mocks.frame={...(mocks.frame as Confirmation),phase:'reverted'};
    return {group:'g',active:[],mapping:null};
  });
});
afterEach(cleanup);
const view=()=>render(<OinkProvider defaultMotion="reduced"><PhysicalConfirmation container={document.body}/></OinkProvider>);
const pending=():Confirmation=>({operation:'operation-visible-peer',phase:'pending',deadline_ms:Date.now()+15000,bounds:{x:0,y:0,w:1920,h:1080},previous_bounds:{x:1920,y:0,w:1920,h:1080},macos:false});
it('does not open a dialog or mutate hardware when this computer has no confirmation',async()=>{
  const v=view();await waitFor(()=>expect(mocks.call).toHaveBeenCalled());
  expect(v.queryByRole('dialog')).toBeNull();
  expect(mocks.call.mock.calls.every(([,p])=>p.command.request.action==='confirmation')).toBe(true);
});
it('confirms from the global window without requiring the monitor settings page',async()=>{
  mocks.frame=pending();const v=view();
  fireEvent.click(await v.findByRole('button',{name:'Keep layout'}));
  await waitFor(()=>expect(mocks.call).toHaveBeenCalledWith('physical_monitors',{command:{action:'experience',request:{action:'keep',operation:'operation-visible-peer'}}}));
  await waitFor(()=>expect(v.queryByRole('button',{name:'Keep layout'})).toBeNull());
});
it('offers a scoped revert while the independent deadline remains armed',async()=>{
  mocks.frame=pending();const v=view();
  fireEvent.click(await v.findByRole('button',{name:'Revert layout'}));
  await waitFor(()=>expect(mocks.call).toHaveBeenCalledWith('physical_monitors',{command:{action:'experience',request:{action:'revert',operation:'operation-visible-peer'}}}));
});
