import { call } from './client';
import type { ScreenKey, Rect } from './types';

export interface MonitorBinding { screen: ScreenKey; input: number }
export interface PhysicalMonitor { name: string; controller_display: string; bindings: MonitorBinding[]; readback?: boolean; initial_input?: number | null }
export interface PhysicalState {
  setup: { revision: number; monitors: PhysicalMonitor[] };
  active: { device: string; display: string; rect: Rect }[];
  controller_available: boolean;
}
export type PhysicalCommand =
  | { action: 'query' | 'clear' }
  | { action: 'save'; expected_revision: number; monitors: PhysicalMonitor[] }
  | { action: 'read'; controller_display: string }
  | { action: 'test'; controller_display: string; input: number; restore_input?: number }
  | { action: 'apply'; expected_revision: number; screens: ScreenKey[]; confirmed_inputs?: Record<string,number> };

export async function physicalCall<R = PhysicalState>(command: PhysicalCommand): Promise<R> {
  const reply = await call<R & { error?: string }>('physical_monitors', { command });
  if (reply?.error) throw new Error(reply.error);
  if (['query', 'save', 'apply', 'clear'].includes(command.action)) {
    const state = reply as unknown as PhysicalState;
    if (!state?.setup || !Array.isArray(state.setup.monitors) || !Array.isArray(state.active))
      throw new Error('This backend does not support physical monitor ownership');
  }
  return reply;
}

/** Cartesian choices preserve physical order: each monitor chooses one binding. */
export function monitorScenes(monitors: PhysicalMonitor[]): ScreenKey[][] {
  if (!monitors.length) return [];
  let scenes: ScreenKey[][] = [[]];
  for (const monitor of monitors) {
    if (!monitor.bindings.length || scenes.length * monitor.bindings.length > 16) return [];
    scenes = scenes.flatMap(scene => monitor.bindings.map(binding => [...scene, binding.screen]));
  }
  return scenes;
}

export const sameScreen = (a: ScreenKey, b: ScreenKey) => a.device === b.device && a.display === b.display;
