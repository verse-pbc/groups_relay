import NDK, {
  NDKEvent,
  NDKPrivateKeySigner,
  NDKRelay,
  NDKRelayAuthPolicies,
  NDKPublishError,
  NDKUser,
} from "@nostr-dev-kit/ndk";
import { nip19 } from "nostr-tools";
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
  readonly rawMessage: string;

  constructor(message: string, context?: string) {
    super(context ? `${context}: ${message}` : message);
    this.name = "NostrGroupError";
    this.rawMessage = message;
  }

  get displayMessage(): string {
    return this.rawMessage;
  }
}

export class NostrClient {
  private ndk: NDK;
  private profileNdk: NDK;
  readonly config: NostrClientConfig;
  private profileCache: Map<string, any> = new Map();

  constructor(key: string, config?: Partial<NostrClientConfig>) {
    try {
      // Get WebSocket URL from environment variable or use current host
      const getWebSocketUrl = () => {
        // Check if we have an environment variable for the WebSocket URL
        if (
          typeof import.meta !== "undefined" &&
          import.meta.env &&
          import.meta.env.VITE_WEBSOCKET_URL
        ) {
          return import.meta.env.VITE_WEBSOCKET_URL;
        }

        // Otherwise, use the current host
        return `${window.location.protocol === "https:" ? "wss:" : "ws:"}//${window.location.host}`;
      };

      const defaultRelayUrl = getWebSocketUrl();
      console.log("NostrClient using relay URL:", defaultRelayUrl);

      this.config = {
        relayUrl: defaultRelayUrl,
        ...config,
      };

      // Validate the key format before creating the signer
      if (!key || typeof key !== "string") {
        throw new Error("Private key is required and must be a string");
      }

      // Try to create the signer with better error handling
      let signer;
      try {
        signer = new NDKPrivateKeySigner(key);
      } catch (signerError) {
        console.error("Failed to create NDKPrivateKeySigner:", signerError);
        throw new Error(
          "Invalid private key provided. Please check the format and try again."
        );
      }

      // Main NDK instance for group operations
      this.ndk = new NDK({
        explicitRelayUrls: [this.config.relayUrl],
        signer,
      });

      // Separate NDK instance for profile fetching
      this.profileNdk = new NDK({
        explicitRelayUrls: ["wss://relay.nos.social", "wss://purplepag.es"],
      });

      this.ndk.pool.on("relay:connect", (relay: NDKRelay) => {
        relay.authPolicy = NDKRelayAuthPolicies.signIn({ ndk: this.ndk });
      });
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to initialize NostrClient"
      );
    }
  }

  get ndkInstance(): NDK {
    return this.ndk;
  }

  async connect() {
    try {
      await Promise.all([this.ndk.connect(), this.profileNdk.connect()]);

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

  async disconnect() {
    try {
      // Close all relay connections from both NDK instances
      const groupRelays = Array.from(this.ndk.pool.relays.values());
      const profileRelays = Array.from(this.profileNdk.pool.relays.values());

      await Promise.all([
        ...groupRelays.map((relay) => relay.disconnect()),
        ...profileRelays.map((relay) => relay.disconnect()),
      ]);

      // Clear any subscriptions
      this.ndk.pool.removeAllListeners();
      this.profileNdk.pool.removeAllListeners();

      console.log("Disconnected from all relays");
    } catch (error) {
      console.error("Error during disconnect:", error);
      throw new NostrGroupError(`Failed to disconnect: ${error}`);
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
          "Please ensure you are authenticated.",
          "No ready relays available"
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
          throw new NostrGroupError(err.message, relay.url);
        }
      }

      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to publish event"
      );
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

  async createGroup(group: Group) {
    // First create the group
    await this.publishEvent(GroupEventKind.CreateGroup, [["h", group.id]]);

    // Then set its metadata
    const metadataTags = [["h", group.id]];
    if (group.name) metadataTags.push(["name", group.name]);
    if (group.about) metadataTags.push(["about", group.about]);
    if (group.picture) metadataTags.push(["picture", group.picture]);
    metadataTags.push([group.private ? "private" : "public"]);
    metadataTags.push([group.closed ? "closed" : "open"]);

    await this.publishEvent(GroupEventKind.EditMetadata, metadataTags);
    return group;
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

  async toggleAdminRole(groupId: string, pubkey: string, isAdmin: boolean) {
    return this.publishEvent(GroupEventKind.PutUser, [
      ["h", groupId],
      ["p", pubkey, isAdmin ? "Admin" : "Member"],
    ]);
  }

  async createInvite(groupId: string, code: string) {
    return this.publishEvent(GroupEventKind.CreateInvite, [
      ["h", groupId],
      ["code", code],
      ["roles", "member"],
    ]);
  }

  async deleteEvent(groupId: string, eventId: string) {
    return this.publishEvent(GroupEventKind.DeleteEvent, [
      ["h", groupId],
      ["e", eventId],
    ]);
  }

  async deleteGroup(groupId: string) {
    return this.publishEvent(GroupEventKind.DeleteGroup, [["h", groupId]]);
  }

  async fetchProfile(pubkey: string) {
    try {
      // Check cache first
      if (this.profileCache.has(pubkey)) {
        return this.profileCache.get(pubkey);
      }

      const user = new NDKUser({ pubkey });
      user.ndk = this.profileNdk; // Use the profile-specific NDK instance
      await user.fetchProfile();

      // Cache the profile
      if (user.profile) {
        this.profileCache.set(pubkey, user.profile);
      }

      return user.profile;
    } catch (error) {
      console.error("Failed to fetch profile:", error);
      return null;
    }
  }

  // Convert a hex pubkey to npub
  pubkeyToNpub(pubkey: string): string {
    try {
      return nip19.npubEncode(pubkey);
    } catch (error) {
      console.error("Failed to convert pubkey to npub:", error);
      return pubkey;
    }
  }

  // Convert an npub to hex pubkey
  npubToPubkey(npub: string): string {
    try {
      const { type, data } = nip19.decode(npub);
      if (type !== "npub") {
        throw new Error("Not an npub");
      }
      return data as string;
    } catch (error) {
      console.error("Failed to convert npub to pubkey:", error);
      throw new NostrGroupError("Invalid npub format");
    }
  }

  // Resolve a NIP-05 address to a pubkey
  async resolveNip05(nip05Address: string): Promise<string> {
    try {
      const [name, domain] = nip05Address.split("@");
      if (!name || !domain) {
        throw new Error("Invalid NIP-05 format");
      }

      const response = await fetch(
        `https://${domain}/.well-known/nostr.json?name=${name}`
      );
      if (!response.ok) {
        throw new Error("Failed to fetch NIP-05 data");
      }

      const data = await response.json();
      const pubkey = data?.names?.[name];
      if (!pubkey) {
        throw new Error("NIP-05 address not found");
      }

      return pubkey;
    } catch (error) {
      console.error("Failed to resolve NIP-05:", error);
      throw new NostrGroupError(
        error instanceof Error ? error.message : "Failed to resolve NIP-05"
      );
    }
  }

  async checkIsRelayAdmin(): Promise<boolean> {
    try {
      const user = await this.ndkInstance.signer?.user();
      if (!user?.pubkey) return false;

      const httpUrl = this.config.relayUrl
        .replace(/^wss?:\/\//, (match) =>
          match === "ws://" ? "http://" : "https://"
        )
        .replace(/\/$/, "");

      const response = await fetch(httpUrl, {
        method: "GET",
        mode: "cors",
        credentials: "omit",
        cache: "no-cache",
        headers: {
          Accept: "application/nostr+json",
          "Cache-Control": "no-cache",
          Pragma: "no-cache",
        },
      });

      const contentType = response.headers.get("content-type");
      if (
        response.ok &&
        (contentType?.includes("application/json") ||
          contentType?.includes("application/nostr+json"))
      ) {
        const relayInfo = await response.json();
        return relayInfo.pubkey === user.pubkey;
      }

      console.warn("Unexpected response type:", contentType);
      return false;
    } catch (error) {
      console.error("Failed to check relay admin status:", error);
      return false;
    }
  }
}

export function hashGroup(group: Group): string {
  const { id, name, invites, joinRequests: join_requests, content } = group;
  return JSON.stringify({ id, name, invites, join_requests, content });
}
