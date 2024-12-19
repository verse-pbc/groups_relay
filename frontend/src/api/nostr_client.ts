import NDK, {
  NDKEvent,
  NDKPrivateKeySigner,
  NDKRelay,
  NDKRelayAuthPolicies,
  NDKPublishError,
} from "@nostr-dev-kit/ndk";
import type { Group } from "../types";

// NIP-29 event kinds
export enum GroupEventKind {
  JoinRequest = 9021,
  LeaveRequest = 9022,
  PutUser = 9000,
  RemoveUser = 9001,
  EditMetadata = 9002,
  DeleteEvent = 9005,
  CreateGroup = 9007,
  DeleteGroup = 9008,
  CreateInvite = 9009,
}

export interface NostrClientConfig {
  relayUrl: string;
}

export class NostrGroupError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "NostrGroupError";
  }
}

export class NostrClient {
  private ndk: NDK;
  readonly config: NostrClientConfig;

  constructor(key: string, config?: Partial<NostrClientConfig>) {
    try {
      this.config = {
        relayUrl: "/ws",
        ...config,
      };

      const signer = new NDKPrivateKeySigner(key);

      this.ndk = new NDK({
        explicitRelayUrls: [this.config.relayUrl],
        signer,
      });

      this.ndk.pool.on("relay:connect", (relay: NDKRelay) => {
        relay.authPolicy = NDKRelayAuthPolicies.signIn({ ndk: this.ndk });
      });
    } catch (error) {
      throw new NostrGroupError(`Failed to initialize NostrClient: ${error}`);
    }
  }

  get ndkInstance(): NDK {
    return this.ndk;
  }

  async connect() {
    try {
      await this.ndk.connect();

      const relays = Array.from(this.ndk.pool.relays.values());
      const firstRelay = await Promise.race(
        relays.map(
          (relay) =>
            new Promise<NDKRelay>((resolve, reject) => {
              // Check if already ready (status 5 = READY)
              if (relay.status === 5) {
                resolve(relay);
                return;
              }

              // Handle connection states
              const handleStatus = () => {
                if (relay.status === 5) {
                  cleanup();
                  resolve(relay);
                }
              };

              // Handle errors
              const handleError = (err: Error) => {
                cleanup();
                reject(err);
              };

              // Setup event listeners
              relay.on("authed", () => {
                cleanup();
                resolve(relay);
              });
              relay.on("disconnect", () =>
                handleError(new Error("Relay disconnected"))
              );
              relay.on("auth:failed", (err) =>
                handleError(new Error(`Auth failed: ${err.message}`))
              );

              const interval = setInterval(handleStatus, 100);

              const cleanup = () => {
                clearInterval(interval);
                relay.removeAllListeners("authed");
                relay.removeAllListeners("disconnect");
                relay.removeAllListeners("auth:failed");
              };

              setTimeout(() => {
                cleanup();
                reject(
                  new Error("Connection timeout waiting for authentication")
                );
              }, 5000);
            })
        )
      );

      console.log(
        "Connected to relays:",
        relays.map((r) => ({
          url: r.url,
          status: r.status === firstRelay.status ? "ready" : r.status,
          connected: r.connected,
        }))
      );
    } catch (error) {
      throw new NostrGroupError(`Failed to connect: ${error}`);
    }
  }

  private async publishEvent(
    kind: GroupEventKind,
    tags: string[][],
    content: string = ""
  ) {
    try {
      // Ensure we have a relay in READY state (status 5)
      const readyRelays = Array.from(this.ndk.pool.relays.values()).filter(
        (r) => r.status === 5
      );

      if (readyRelays.length === 0) {
        throw new NostrGroupError(
          "No ready relays available. Please ensure you are authenticated."
        );
      }

      const ndkEvent = new NDKEvent(this.ndk);
      ndkEvent.kind = kind;
      ndkEvent.tags = tags;
      ndkEvent.content = content;
      await ndkEvent.sign();
      console.log("ndkEvent", JSON.stringify(ndkEvent.rawEvent()));

      const publishResult = await ndkEvent.publish();
      console.log("Event published successfully:", !!publishResult);

      return ndkEvent;
    } catch (error) {
      // If it's a NDKPublishError, we can get specific relay errors
      if (error instanceof NDKPublishError) {
        for (const [relay, err] of error.errors) {
          throw new NostrGroupError(`${relay.url}: ${err.message}`);
        }
      }

      throw new NostrGroupError(`Failed to publish event: ${error}`);
    }
  }

  async sendJoinRequest(groupId: string, inviteCode?: string) {
    const tags = [["h", groupId]];
    if (inviteCode) {
      tags.push(["code", inviteCode]);
    }
    return this.publishEvent(GroupEventKind.JoinRequest, tags);
  }

  async acceptJoinRequest(groupId: string, pubkey: string) {
    return this.publishEvent(GroupEventKind.PutUser, [
      ["h", groupId],
      ["p", pubkey, "member"],
    ]);
  }

  async createGroup(
    groupId: string,
    name: string,
    about: string = "",
    picture: string = ""
  ) {
    // First create the group
    await this.publishEvent(GroupEventKind.CreateGroup, [["h", groupId]]);

    // Then set its metadata
    const metadataTags = [["h", groupId]];
    if (name) metadataTags.push(["name", name]);
    if (about) metadataTags.push(["about", about]);
    if (picture) metadataTags.push(["picture", picture]);
    metadataTags.push(["public"]);
    metadataTags.push(["open"]);

    return this.publishEvent(GroupEventKind.EditMetadata, metadataTags);
  }

  async updateGroupName(groupId: string, newName: string) {
    return this.publishEvent(GroupEventKind.EditMetadata, [
      ["h", groupId],
      ["name", newName],
    ]);
  }

  async updateGroupMetadata(group: Group) {
    const tags = [["h", group.id]];
    if (group.name) tags.push(["name", group.name]);
    if (group.picture) tags.push(["picture", group.picture]);
    if (group.about) tags.push(["about", group.about]);
    tags.push([group.private ? "private" : "public"]);
    tags.push([group.closed ? "closed" : "open"]);

    return this.publishEvent(GroupEventKind.EditMetadata, tags);
  }

  async leaveGroup(groupId: string) {
    return this.publishEvent(GroupEventKind.LeaveRequest, [["h", groupId]]);
  }

  async addModerator(groupId: string, pubkey: string) {
    return this.publishEvent(GroupEventKind.PutUser, [
      ["h", groupId],
      ["p", pubkey, "moderator"],
    ]);
  }

  async removeModerator(groupId: string, pubkey: string) {
    return this.publishEvent(GroupEventKind.RemoveUser, [
      ["h", groupId],
      ["p", pubkey],
    ]);
  }

  async removeMember(groupId: string, pubkey: string) {
    return this.publishEvent(GroupEventKind.RemoveUser, [
      ["h", groupId],
      ["p", pubkey],
    ]);
  }

  async addMember(groupId: string, pubkey: string) {
    return this.publishEvent(GroupEventKind.PutUser, [
      ["h", groupId],
      ["p", pubkey, "member"],
    ]);
  }

  async createInvite(groupId: string, code: string) {
    return this.publishEvent(GroupEventKind.CreateInvite, [
      ["h", groupId],
      ["code", code],
      ["roles", "member"],
    ]);
  }
}

export function hashGroup(group: Group): string {
  const { id, name, invites, joinRequests: join_requests, content } = group;
  return JSON.stringify({ id, name, invites, join_requests, content });
}
