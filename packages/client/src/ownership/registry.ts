import type { OwnershipAdapter } from "./types.js";

/** Small registry for application-selected Ownership proof adapters. */
export class OwnershipAdapterRegistry {
  private readonly adapters = new Map<string, OwnershipAdapter>();

  register(adapter: OwnershipAdapter): void {
    if (!adapter.id || adapter.id.trim() !== adapter.id) {
      throw new Error("Ownership adapter id must be a non-empty canonical string");
    }
    if (this.adapters.has(adapter.id)) {
      throw new Error(`Ownership adapter is already registered: ${adapter.id}`);
    }
    this.adapters.set(adapter.id, adapter);
  }

  get(adapterId: string): OwnershipAdapter {
    const adapter = this.adapters.get(adapterId);
    if (!adapter) throw new Error(`unknown Ownership adapter: ${adapterId}`);
    return adapter;
  }

  verify(adapterId: string, subject: Parameters<OwnershipAdapter["verify"]>[0], controller: Parameters<OwnershipAdapter["verify"]>[1], proof: Uint8Array): boolean {
    return this.get(adapterId).verify(subject, controller, proof);
  }
}
