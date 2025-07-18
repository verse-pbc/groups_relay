import { Component } from "preact";
import { NostrClient, GroupEventKind } from "../api/nostr_client";
import type { Group, GroupContent as GroupChatMessage } from "../types";
import { CreateGroupForm } from "./CreateGroupForm";
import { GroupCard } from "./GroupCard";
import { FlashMessage } from "./FlashMessage";
import { GroupSidebar } from "./GroupSidebar";
import { BurgerButton } from "./BurgerButton";
import { ProfileMenu } from "./ProfileMenu";
import { SubdomainList } from "./SubdomainList";
// WalletDisplay moved to ProfileMenu

// Import NDK types if possible from your client setup, otherwise define them
// Assuming they might be available like this (adjust if needed):
// import type { NDK, NDKEvent, NDKFilter, NDKKind } from "@nostr-dev-kit/ndk";
// If not available via import, define NDKKind:
// --- Start Local Type Definitions ---
type NDKKind = number;
type NDKFilter = Record<string, any>; // Basic type
type NDKEvent = { // Define structure based on usage
    id: string;
    kind: NDKKind;
    pubkey: string;
    created_at: number;
    content: string;
    tags: string[][];
    sig?: string; // Make sig optional
    rawEvent: () => any; // Function returning the raw event data
};
// Remove unused NDK type
// type NDK = Record<string, any>;
// --- End Local Type Definitions ---

// Metadata kinds - loaded immediately for all groups
const metadataKinds: NDKKind[] = [
  39000, // Group metadata
  39001, // Group admins  
  39002, // Group members
  GroupEventKind.CreateGroup, // Group creation events to get IDs
];

// Content kinds - loaded on-demand per group
// h-tagged events (regular events)
const hTaggedContentKinds: NDKKind[] = [
  GroupEventKind.CreateInvite, // 9009 - invites use h tags
  GroupEventKind.JoinRequest,  // 9021 - join requests use h tags
  GroupEventKind.PutUser,      // 9000 - user management uses h tags
  GroupEventKind.RemoveUser,   // 9001 - user removal uses h tags
  9, // Chat message
  11, // DM (Note: DMs might require specific handling/decryption not shown here)
];

// d-tagged events (addressable events 30000+)
const dTaggedContentKinds: NDKKind[] = [
  // Currently no addressable content events, but keeping structure for future
  // 39000+ events are in metadataKinds, not content
];

// Define all kinds to fetch historically and subscribe to live
// const relevantKinds: NDKKind[] = [
//   ...metadataKinds,
//   GroupEventKind.CreateGroup,
//   ...hTaggedContentKinds,
//   ...dTaggedContentKinds
// ];
export interface FlashMessageData {
  message: string;
  type: "success" | "error" | "info";
}

interface AppProps {
  client: NostrClient; // Assuming NostrClient exposes the NDK instance
  onLogout: () => void;
}

interface AppState {
  groups: Group[];
  flashMessage: FlashMessageData | null;
  groupsMap: Map<string, Group>;
  selectedGroup: Group | null;
  isMobileMenuOpen: boolean;
  pendingGroupSelection: string | null; // Queue of one for simplicity
  isLoadingHistory: boolean; // Added state to indicate loading
  currentSubdomain: string | null; // Current subdomain being viewed
}

export class App extends Component<AppProps, AppState> {
  // No cleanup needed for historical fetch using fetchEvents
  private liveSubscriptionCleanup: (() => void) | null = null; // For the live subscription
  private groupContentSubscriptions: Map<string, () => void> = new Map(); // Track per-group subscriptions

  constructor(props: AppProps) {
    super(props);
    this.state = {
      groups: [],
      flashMessage: null,
      groupsMap: new Map(),
      selectedGroup: null,
      isMobileMenuOpen: false,
      pendingGroupSelection: null,
      isLoadingHistory: true, // Start in loading state
      currentSubdomain: this.extractCurrentSubdomain(),
    };
  }

  // Extract subdomain from current URL
  private extractCurrentSubdomain(): string | null {
    const { hostname } = window.location;
    
    // For localhost or IP addresses, no subdomain
    if (hostname === 'localhost' || hostname.match(/^\d+\.\d+\.\d+\.\d+$/)) {
      return null;
    }
    
    const parts = hostname.split('.');
    // Assume base domain is last 2 parts (e.g., example.com)
    // If there are more than 2 parts, everything before the last 2 is subdomain
    if (parts.length > 2) {
      const subdomain = parts.slice(0, -2).join('.');
      return subdomain;
    }
    
    return null;
  }

  private getOrCreateGroup = (
    groupId: string,
    createdAt: number,
    groupsMap: Map<string, Group>
  ): Group => {
    const existingGroup = groupsMap.get(groupId);
    if (!existingGroup) {
      const group: Group = {
        id: groupId,
        name: "",
        about: "",
        picture: "",
        private: false,
        closed: false,
        broadcast: false,
        created_at: 0, // Initialize, set by CreateGroup event
        updated_at: createdAt, // Track latest interaction
        members: [],
        invites: {},
        joinRequests: [],
        content: [],
      };
      return group;
    }

    // Return existing group but update the 'updated_at' timestamp
    return {
      ...existingGroup,
      updated_at: Math.max(existingGroup.updated_at, createdAt),
    };
  };

  // Modified processEvent to mutate the map passed in, or ensure it's reassigned
  processEvent = (event: any, groupsMap: Map<string, Group>): Map<string, Group> => {
      const groupId = event.tags.find((t: string[]) => t[0] === "h" || t[0] === "d")?.[1];
      if (!groupId) return groupsMap; // Return unchanged map if no group ID

      const group = this.getOrCreateGroup(groupId, event.created_at, groupsMap);

      // Ensure the group exists in the map before proceeding
      if (!groupsMap.has(groupId)) {
          groupsMap.set(groupId, group);
      }

      // Create a mutable copy of the group's state if needed, but modifying the map entry directly is okay
      const baseGroup = groupsMap.get(groupId)!; // We know it exists now

      // Create copies of arrays/objects that will be modified to avoid direct state mutation issues
      // if not mutating the map entry directly. If mutating map entry, this spread is just for safety.
      const mutableGroup = {
          ...baseGroup,
          members: [...baseGroup.members],
          joinRequests: [...baseGroup.joinRequests],
          invites: { ...baseGroup.invites },
          content: baseGroup.content ? [...baseGroup.content] : []
      };

      let updated = false; // Flag to check if we actually changed the group

      switch (event.kind as NDKKind) {
          case GroupEventKind.CreateGroup: {
              mutableGroup.created_at = event.created_at;
              updated = true;
              break;
          }

          case GroupEventKind.CreateInvite: {
              const codeTag = event.tags.find((t: string[]) => t[0] === "code");
              if (codeTag) {
                  const [_, code] = codeTag;
                  // Ensure invites object exists
                  mutableGroup.invites = mutableGroup.invites || {};
                  mutableGroup.invites[code] = {
                      code,
                      pubkey: event.pubkey,
                      roles: ["member"], // Default role
                      id: event.id
                  };
                  updated = true;
              }
              break;
          }

          case 39000: { // Group metadata
              const newMetadata: Partial<Group> = { broadcast: false }; // Default broadcast to false
              let metadataChanged = false;

              for (const tagArr of event.tags) {
                  const tag = tagArr[0];
                  const value = tagArr[1];
                  let changed = true; // Assume change unless value is same

                  switch (tag) {
                      case "name": if (mutableGroup.name !== value) mutableGroup.name = value; else changed = false; break;
                      case "about": if (mutableGroup.about !== value) mutableGroup.about = value; else changed = false; break;
                      case "picture": if (mutableGroup.picture !== value) mutableGroup.picture = value; else changed = false; break;
                      case "private": if (!mutableGroup.private) mutableGroup.private = true; else changed = false; break;
                      case "public": if (mutableGroup.private) mutableGroup.private = false; else changed = false; break;
                      case "closed": if (!mutableGroup.closed) mutableGroup.closed = true; else changed = false; break;
                      case "open": if (mutableGroup.closed) mutableGroup.closed = false; else changed = false; break;
                      case "broadcast": if (!mutableGroup.broadcast) mutableGroup.broadcast = true; else changed = false; break;
                      default: changed = false; // Ignore unknown tags for change tracking
                  }
                  if (changed) metadataChanged = true;
              }
              // Apply the collected metadata changes if any occurred
              if (metadataChanged) {
                  Object.assign(mutableGroup, newMetadata); // Apply changes
                  updated = true;
              }
              break;
          }

          case 39001: { // Group admins - NIP-29: AUTHORITATIVE list of admins
              // According to NIP-29, kind 39001 is the authoritative list of admins
              // This event completely replaces the admin list - not additive
              
              const eventAdmins = new Set(
                  event.tags
                      .filter((t: string[]) => t[0] === "p")
                      .map((t: string[]) => t[1]) // Just get pubkeys
              );

              let membersChanged = false;
              
              // Process all members to update admin status
              const updatedMembers = mutableGroup.members.map(member => {
                  const memberCopy = { ...member };
                  const hasAdminRole = member.roles.some(r => r.toLowerCase() === 'admin');
                  const shouldBeAdmin = eventAdmins.has(member.pubkey);
                  
                  if (shouldBeAdmin && !hasAdminRole) {
                      // Add Admin role
                      memberCopy.roles = [...member.roles, 'Admin'];
                      membersChanged = true;
                  } else if (!shouldBeAdmin && hasAdminRole) {
                      // Remove Admin role
                      memberCopy.roles = member.roles.filter(r => r.toLowerCase() !== 'admin');
                      if (memberCopy.roles.length === 0) {
                          memberCopy.roles = ['Member'];
                      }
                      membersChanged = true;
                  }
                  
                  return memberCopy;
              });
              
              // Add any admins that aren't in the members list yet
              for (const adminPubkey of eventAdmins) {
                  if (!updatedMembers.some(m => m.pubkey === adminPubkey)) {
                      updatedMembers.push({
                          pubkey: adminPubkey as string,
                          roles: ['Admin', 'Member']
                      });
                      membersChanged = true;
                  }
              }

              if (membersChanged) {
                  mutableGroup.members = updatedMembers;
                  updated = true;
              }
              break;
          }


          case 39002: { // Group members metadata - NIP-29: Lists all members WITHOUT roles
              // According to NIP-29, kind 39002 lists all members (including admins) but without role information
              // We need to preserve existing roles while updating the member list
              
              const eventMemberPubkeys = new Set<string>(
                  event.tags
                      .filter((t: string[]) => t[0] === "p")
                      .map((t: string[]) => t[1])
              );

              // Create a map of existing members to preserve their roles
              const existingMembersMap = new Map(
                  mutableGroup.members.map(m => [m.pubkey, m])
              );

              const newMemberList: { pubkey: string; roles: string[] }[] = [];
              
              // Add all members from the event, preserving existing roles
              for (const pubkey of eventMemberPubkeys) {
                  const existingMember = existingMembersMap.get(pubkey);
                  if (existingMember) {
                      // Keep existing roles
                      newMemberList.push({ ...existingMember });
                  } else {
                      // New member, default to Member role
                      newMemberList.push({ pubkey, roles: ["Member"] });
                  }
              }

              // Check if the member list actually changed
              const existingPubkeys = new Set(mutableGroup.members.map(m => m.pubkey));
              const membersChanged = eventMemberPubkeys.size !== existingPubkeys.size ||
                  [...eventMemberPubkeys].some(pk => !existingPubkeys.has(pk));

              if (membersChanged) {
                  mutableGroup.members = newMemberList;
                  // Clear join requests for users who are now members
                   mutableGroup.joinRequests = mutableGroup.joinRequests.filter(pubkey =>
                       !newMemberList.some(m => m.pubkey === pubkey)
                   );
                  updated = true;
              }
              break;
          }

          case 9: // Chat message
          case 11: // DM
          {
              const content: GroupChatMessage = {
                  id: event.id,
                  pubkey: event.pubkey,
                  kind: event.kind,
                  content: event.content, // Assuming plain text, decryption might be needed for kind 11 or encrypted kind 9
                  created_at: event.created_at,
              };

              // Avoid duplicates and sort
              const contentMap = new Map(mutableGroup.content.map(c => [c.id, c]));
              if (!contentMap.has(content.id)) {
                  contentMap.set(content.id, content);
                  mutableGroup.content = Array.from(contentMap.values())
                      .sort((a, b) => a.created_at - b.created_at) // Oldest first
                      .slice(-100); // Keep last 100 messages (adjust limit as needed)
                  updated = true;
              }
              break;
          }

          case GroupEventKind.JoinRequest: {
              // Add join request if user is not already a member
              if (!mutableGroup.members.some(member => member.pubkey === event.pubkey)) {
                  if (!mutableGroup.joinRequests.includes(event.pubkey)) {
                      mutableGroup.joinRequests = [...mutableGroup.joinRequests, event.pubkey];
                      updated = true;
                  }
              }
              break;
          }

          case GroupEventKind.PutUser: {
              // Handle adding/updating user roles
              // NOTE: Kind 9000 events should not add Admin roles - those come from 39001
              const userTag = event.tags.find((t: string[]) => t[0] === "p");
              if (userTag) {
                  const pubkey = userTag[1];
                  const newRoles = userTag.slice(2).length > 0 ? userTag.slice(2) : ["Member"];
                  
                  // Normalize and filter roles - remove any admin roles from Kind 9000
                  const normalizedNew = newRoles.map((r: string) => {
                      const lower = r.toLowerCase();
                      return lower === 'admin' ? 'Admin' : lower === 'member' ? 'Member' : r;
                  }).filter((r: string) => r.toLowerCase() !== 'admin'); // Filter out admin roles from Kind 9000
                  
                  // If no roles left after filtering, default to Member
                  if (normalizedNew.length === 0) {
                      normalizedNew.push('Member');
                  }
                  
                  // Find existing member or create new one
                  const existingMemberIndex = mutableGroup.members.findIndex(m => m.pubkey === pubkey);
                  
                  if (existingMemberIndex >= 0) {
                      // Get existing roles and preserve Admin if they have it
                      const existingRoles = mutableGroup.members[existingMemberIndex].roles;
                      const hasAdmin = existingRoles.some(r => r.toLowerCase() === 'admin');
                      
                      // Build new role set
                      const roleSet = new Set<string>(normalizedNew);
                      
                      // Preserve admin role if they already have it
                      if (hasAdmin) {
                          roleSet.add('Admin');
                      }
                      
                      mutableGroup.members[existingMemberIndex].roles = Array.from(roleSet);
                  } else {
                      // Add new member (without admin role - that comes from 39001)
                      mutableGroup.members.push({ pubkey, roles: normalizedNew });
                  }
                  
                  // Remove from join requests if they're being added
                  mutableGroup.joinRequests = mutableGroup.joinRequests.filter(p => p !== pubkey);
                  
                  updated = true;
              }
              break;
          }

          case GroupEventKind.RemoveUser: {
              // Handle removing users from the group
              const userTag = event.tags.find((t: string[]) => t[0] === "p");
              if (userTag) {
                  const pubkey = userTag[1];
                  mutableGroup.members = mutableGroup.members.filter(m => m.pubkey !== pubkey);
                  updated = true;
              }
              break;
          }

          // Add case for Kind 5 (Deletion) if you want to handle message deletions
          // case 5: {
          //   const deletedEventIds = event.tags.filter(t => t[0] === 'e').map(t => t[1]);
          //   const initialContentLength = mutableGroup.content.length;
          //   mutableGroup.content = mutableGroup.content.filter(c => !deletedEventIds.includes(c.id));
          //   if (mutableGroup.content.length !== initialContentLength) {
          //       updated = true;
          //   }
          //   break;
          // }

          default: {
              // Ignore unknown kinds for processing, but the group was touched so update timestamp
              // No 'updated = true' here unless the timestamp logic below handles it.
              break;
          }
      }

      // Always update the group's updated_at timestamp if an event related to it was processed
      mutableGroup.updated_at = Math.max(baseGroup.updated_at, event.created_at);

      // If any part of the group was updated, or just the timestamp changed, update the map entry
      if (updated || mutableGroup.updated_at !== baseGroup.updated_at) {
          groupsMap.set(groupId, mutableGroup);
      }

      return groupsMap; // Return the potentially modified map
  };


  // Fetch only group metadata (39000, 39001, 39002, and creation events)
  private async fetchGroupMetadata(): Promise<{ groupsMap: Map<string, Group>; latestTimestamp: number }> {
      let groupsMap = new Map<string, Group>();
      let latestTimestamp = 0;
      
      try {
          // Fetch all metadata events at once (they should be relatively few)
          const filter: NDKFilter = { 
              kinds: metadataKinds,
              // No limit - we want all metadata
          };
          
          const events = await this.props.client.ndkInstance.fetchEvents(filter) as Set<NDKEvent>;
          
          events.forEach((event: NDKEvent) => {
              if (typeof event.created_at === 'number') {
                  groupsMap = this.processEvent(event.rawEvent(), groupsMap);
                  latestTimestamp = Math.max(latestTimestamp, event.created_at);
              }
          });
          
      } catch (error) {
          throw error;
      }
      
      return { groupsMap, latestTimestamp };
  }
  
  // Subscribe to live content updates for a specific group
  private subscribeToGroupContent(groupId: string): void {
      // Unsubscribe from existing subscription if any
      const existingSub = this.groupContentSubscriptions.get(groupId);
      if (existingSub) {
          existingSub();
          this.groupContentSubscriptions.delete(groupId);
      }
      
      // Subscribe to both h-tagged and d-tagged events separately
      const subscriptions: any[] = [];
      
      // Subscribe to h-tagged events (regular events like invites, messages, etc)
      if (hTaggedContentKinds.length > 0) {
          const hSub = this.props.client.ndkInstance.subscribe(
              { 
                  kinds: hTaggedContentKinds,
                  "#h": [groupId],
                  since: Math.floor(Date.now() / 1000) // Only new events
              },
              { closeOnEose: false }
          );
          subscriptions.push(hSub);
      }
      
      // Subscribe to d-tagged events (addressable events) if any
      if (dTaggedContentKinds.length > 0) {
          const dSub = this.props.client.ndkInstance.subscribe(
              { 
                  kinds: dTaggedContentKinds,
                  "#d": [groupId],
                  since: Math.floor(Date.now() / 1000) // Only new events
              },
              { closeOnEose: false }
          );
          subscriptions.push(dSub);
      }
      
      // Add event handlers to all subscriptions
      const eventHandler = (event: NDKEvent) => {
          this.setState((prevState) => {
              let newGroupsMap = new Map(prevState.groupsMap);
              newGroupsMap = this.processEvent(event.rawEvent(), newGroupsMap);
              
              const sortedGroups = Array.from(newGroupsMap.values()).sort(
                  (a, b) => b.updated_at - a.updated_at
              );
              
              // Update selected group if it's the one receiving content
              let newSelectedGroup = prevState.selectedGroup;
              if (prevState.selectedGroup?.id === groupId) {
                  newSelectedGroup = newGroupsMap.get(groupId) || null;
              }
              
              return {
                  groupsMap: newGroupsMap,
                  groups: sortedGroups,
                  selectedGroup: newSelectedGroup
              };
          });
      };
      
      subscriptions.forEach(sub => {
          sub.on("event", eventHandler);
      });
      
      // Store cleanup function
      const cleanup = () => {
          subscriptions.forEach(sub => sub.stop());
      };
      
      this.groupContentSubscriptions.set(groupId, cleanup);
  }

  // Helper function to add timeout to promises
  private withTimeout<T>(promise: Promise<T>, timeoutMs: number, errorMessage: string): Promise<T> {
      return Promise.race([
          promise,
          new Promise<T>((_, reject) => 
              setTimeout(() => reject(new Error(errorMessage)), timeoutMs)
          )
      ]);
  }

  // Fetch full content for a specific group
  private async fetchGroupContent(groupId: string): Promise<void> {
      const group = this.state.groupsMap.get(groupId);
      if (!group) {
          console.log(`fetchGroupContent: Group ${groupId} not found`);
          return;
      }
      
      // If already loaded or loading, skip
      if (group.isFullyLoaded || group.isLoading) {
          console.log(`fetchGroupContent: Group ${groupId} already loaded or loading`, { isFullyLoaded: group.isFullyLoaded, isLoading: group.isLoading });
          return;
      }
      
      console.log(`fetchGroupContent: Starting to load content for group ${groupId}`);
      
      // Update loading state
      this.updateGroupsMap((map) => {
          const g = map.get(groupId);
          if (g) {
              g.isLoading = true;
              g.loadError = undefined;
          }
      });
      
      try {
          // Ensure we have at least one connected relay
          const pool = this.props.client.ndkInstance.pool;
          const allRelays = Array.from(pool.relays.values());
          // NDK relay status: 0=disconnected, 1=connecting, 2=connected, 3=reconnecting, 4=error, 5=authenticated
          const connectedRelays = allRelays.filter(r => r.status === 2 || r.status === 5);
          
          console.log(`fetchGroupContent: Relay status check:`, {
              totalRelays: allRelays.length,
              connectedRelays: connectedRelays.length,
              relayStates: allRelays.map(r => ({ url: r.url, status: r.status, statusName: ['disconnected', 'connecting', 'connected', 'reconnecting', 'error', 'authenticated'][r.status] }))
          });
          
          if (connectedRelays.length === 0) {
              console.log(`fetchGroupContent: No connected relays, attempting to reconnect...`);
              
              // Try to reconnect
              try {
                  await this.props.client.connect();
                  console.log(`fetchGroupContent: Reconnection attempt completed`);
              } catch (reconnectError) {
                  console.error(`fetchGroupContent: Reconnection failed:`, reconnectError);
              }
              
              // Check relay status after reconnection attempt
              const relaysAfterReconnect = Array.from(pool.relays.values()).filter(r => r.status === 2 || r.status === 5);
              console.log(`fetchGroupContent: After reconnection - connected relays: ${relaysAfterReconnect.length}`);
              
              if (relaysAfterReconnect.length === 0) {
                  throw new Error('No relays connected after reconnection attempt');
              }
          }
          
          // Wait for authentication if relay is connected but not authenticated
          const authenticatedRelays = allRelays.filter(r => r.status === 5);
          if (authenticatedRelays.length === 0) {
              console.log(`fetchGroupContent: No authenticated relays, waiting for authentication...`);
              // Wait a bit for authentication to complete
              await new Promise(resolve => setTimeout(resolve, 1000));
              
              const authenticatedAfterWait = Array.from(pool.relays.values()).filter(r => r.status === 5);
              console.log(`fetchGroupContent: After auth wait - authenticated relays: ${authenticatedAfterWait.length}`);
          }
          // Fetch content events for this group using separate filters for h-tagged and d-tagged events
          const allEvents = new Set<NDKEvent>();
          
          // Fetch h-tagged events (regular events like invites, messages, etc)
          if (hTaggedContentKinds.length > 0) {
              const hFilter: NDKFilter = {
                  kinds: hTaggedContentKinds,
                  "#h": [groupId],
                  limit: 250, // Split limit between the two filters
              };
              
              console.log(`fetchGroupContent: Fetching h-tagged events with filter:`, hFilter);
              
              // Check relay connection status
              const relays = Array.from(this.props.client.ndkInstance.pool.relays.values());
              console.log(`fetchGroupContent: Relay pool status:`, {
                  relayCount: relays.length,
                  relays: relays.map(r => ({
                      url: r.url,
                      status: r.status,
                      authenticated: r.status === 5,
                      connected: r.status === 2 || r.status === 5
                  }))
              });
              
              const hEvents = await this.withTimeout(
                  this.props.client.ndkInstance.fetchEvents(hFilter) as Promise<Set<NDKEvent>>,
                  10000,
                  'Timeout fetching h-tagged events'
              );
              console.log(`fetchGroupContent: Got ${hEvents.size} h-tagged events`);
              hEvents.forEach(event => allEvents.add(event));
          }
          
          // Fetch d-tagged events (addressable events) if any
          if (dTaggedContentKinds.length > 0) {
              const dFilter: NDKFilter = {
                  kinds: dTaggedContentKinds,
                  "#d": [groupId],
                  limit: 250, // Split limit between the two filters
              };
              
              console.log(`fetchGroupContent: Fetching d-tagged events with filter:`, dFilter);
              const dEvents = await this.withTimeout(
                  this.props.client.ndkInstance.fetchEvents(dFilter) as Promise<Set<NDKEvent>>,
                  10000,
                  'Timeout fetching d-tagged events'
              );
              console.log(`fetchGroupContent: Got ${dEvents.size} d-tagged events`);
              dEvents.forEach(event => allEvents.add(event));
          }
          
          const events = allEvents;
          console.log(`fetchGroupContent: Fetched ${events.size} events for group ${groupId}`);
          
          // Process events and update group
          this.setState((prevState) => {
              let newGroupsMap = new Map(prevState.groupsMap);
              
              events.forEach((event: NDKEvent) => {
                  if (typeof event.created_at === 'number') {
                      newGroupsMap = this.processEvent(event.rawEvent(), newGroupsMap);
                  }
              });
              
              // Mark as fully loaded
              const updatedGroup = newGroupsMap.get(groupId);
              if (updatedGroup) {
                  updatedGroup.isLoading = false;
                  updatedGroup.isFullyLoaded = true;
                  console.log(`fetchGroupContent: Marked group ${groupId} as fully loaded`);
              }
              
              const sortedGroups = Array.from(newGroupsMap.values()).sort(
                  (a, b) => b.updated_at - a.updated_at
              );
              
              return {
                  groupsMap: newGroupsMap,
                  groups: sortedGroups,
                  selectedGroup: prevState.selectedGroup?.id === groupId ? 
                      newGroupsMap.get(groupId) || prevState.selectedGroup : 
                      prevState.selectedGroup
              };
          });
          
          console.log(`fetchGroupContent: Successfully loaded content for group ${groupId}`);
          
      } catch (error) {
          console.error(`fetchGroupContent: Error loading group ${groupId}:`, error);
          
          let errorMessage = 'Failed to load group content';
          if (error instanceof Error) {
              errorMessage = error.message;
              // Check if this is an access denied error
              if (errorMessage.includes('Timeout') && groupId) {
                  // This might be an access denied issue, check if group is private
                  const group = this.state.groupsMap.get(groupId);
                  if (group?.private) {
                      errorMessage = 'Access denied: You are not a member of this private group';
                  }
              }
          }
          
          // Update error state
          this.updateGroupsMap((map) => {
              const g = map.get(groupId);
              if (g) {
                  g.isLoading = false;
                  g.loadError = errorMessage;
              }
          });
      }
  }

  // New method for paginated historical fetch
  /*
  private async fetchHistoricalDataPaginated(
      kinds: NDKKind[],
      batchSize: number
  ): Promise<{ groupsMap: Map<string, Group>; latestTimestamp: number }> {
      let groupsMap = new Map<string, Group>(); // Initialize map locally for this fetch
      let oldestTimestampInBatch = Math.floor(Date.now() / 1000); // Start from now
      let newestHistoricalTimestamp = 0;
      let continueFetching = true;
      let totalFetched = 0;

      console.log("Starting paginated historical fetch...");

      while (continueFetching) {
          // Prevent infinite loops in case of unexpected issues
           if (totalFetched > 50000) { // Safety break after fetching a large number of events
              console.warn("Safety break triggered: fetched over 50,000 events during pagination.");
              break;
           }

          const filter: NDKFilter = { kinds: kinds, limit: batchSize, until: oldestTimestampInBatch };
          console.log(`Fetching batch with filter: limit=${batchSize}, until=${oldestTimestampInBatch}`);

          try {
              // Use the NDK instance from props
              // Cast the result to Set<NDKEvent> based on our local type
              const events = await this.props.client.ndkInstance.fetchEvents(filter) as Set<NDKEvent>;
              totalFetched += events.size;

              if (events.size === 0) {
                  console.log("No more events found for this filter range. Stopping.");
                  continueFetching = false;
                  break;
              }

              let batchOldest = oldestTimestampInBatch;
              let batchNewest = 0;

              events.forEach((event: NDKEvent) => {
                  // Ensure event.created_at exists and is a number
                  if (typeof event.created_at === 'number') {
                      // Process event using the class method, updating the local map
                       groupsMap = this.processEvent(event.rawEvent(), groupsMap); // Reassign map

                      batchOldest = Math.min(batchOldest, event.created_at);
                      batchNewest = Math.max(batchNewest, event.created_at);

                       // Update the overall newest timestamp seen so far
                       newestHistoricalTimestamp = Math.max(newestHistoricalTimestamp, event.created_at);
                  } else {
                      console.warn("Event received without valid created_at:", event.id, event.rawEvent());
                  }
              });


              console.log(`Fetched ${events.size} events. Oldest in batch: ${batchOldest}, Newest overall: ${newestHistoricalTimestamp}`);

              if (events.size < batchSize) {
                  console.log(`Fetched ${events.size} events (less than batch size ${batchSize}). Assuming end of history.`);
                  continueFetching = false;
              } else {
                  // If we got a full batch and the oldest didn't change, something is wrong (e.g., duplicate timestamps exactly at the boundary)
                  // Prevent infinite loop by stopping. Or could try decrementing timestamp by 1.
                  if (oldestTimestampInBatch === batchOldest) {
                      console.warn(`Oldest timestamp (${batchOldest}) did not change after fetching a full batch. Decrementing by 1 to continue.`);
                      oldestTimestampInBatch = batchOldest - 1;
                  } else {
                      oldestTimestampInBatch = batchOldest;
                  }

                  // Optional delay between fetches to be kind to relays
                  // await new Promise(resolve => setTimeout(resolve, 100));
              }
          } catch (error) {
              console.error("Error fetching batch:", error);
              // Decide how to handle errors - maybe retry once, or stop?
              continueFetching = false; // Stop on error for simplicity
          }
      }

      console.log(`Paginated fetch complete. ${totalFetched} events fetched. ${groupsMap.size} groups processed. Newest historical timestamp: ${newestHistoricalTimestamp}`);
      return { groupsMap, latestTimestamp: newestHistoricalTimestamp };
  }
  */


  async componentDidMount() {
    this.setState({ isLoadingHistory: true });

    try {
      // Phase 1: Fetch only metadata
      const { groupsMap: initialGroupsMap, latestTimestamp: metadataTimestamp } =
        await this.fetchGroupMetadata();

      // Update state with metadata-only groups
      const sortedGroups = Array.from(initialGroupsMap.values()).sort(
        (a, b) => b.updated_at - a.updated_at
      );
      
      const pendingUpdate = this.checkPendingSelection(initialGroupsMap);
      const initialSelectedGroup = pendingUpdate?.selectedGroup || 
        (sortedGroups.length > 0 ? sortedGroups[0] : null);

      this.setState({
        groupsMap: initialGroupsMap,
        groups: sortedGroups,
        selectedGroup: initialSelectedGroup,
        isLoadingHistory: false,
        ...(pendingUpdate || {})
      }, async () => {
        // Phase 2: Load content for selected group
        if (initialSelectedGroup) {
          await this.fetchGroupContent(initialSelectedGroup.id);
          // Prefetch member data after content loads
          const updatedGroup = this.state.groupsMap.get(initialSelectedGroup.id);
          if (updatedGroup) {
            this.prefetchGroupMemberData(updatedGroup);
          }
        }
        
        // Phase 3: Start live subscription for metadata
        const metadataSub = this.props.client.ndkInstance.subscribe(
          { kinds: metadataKinds, since: metadataTimestamp + 1 },
          { closeOnEose: false }
        );

        metadataSub.on("event", (event: NDKEvent) => {
          this.setState((prevState) => {
              let newGroupsMap = new Map(prevState.groupsMap);
              newGroupsMap = this.processEvent(event.rawEvent(), newGroupsMap);

              const sortedGroups = Array.from(newGroupsMap.values()).sort(
                  (a, b) => b.updated_at - a.updated_at
              );

              const pendingUpdate = this.checkPendingSelection(newGroupsMap);

              // Update selected group reference if it still exists
              const currentSelectedId = prevState.selectedGroup?.id;
              let newSelectedGroup = prevState.selectedGroup;
              if (currentSelectedId) {
                  newSelectedGroup = newGroupsMap.get(currentSelectedId) || null;
              }
              if (pendingUpdate?.selectedGroup) {
                  newSelectedGroup = pendingUpdate.selectedGroup;
              } else if (!newSelectedGroup && sortedGroups.length === 1) {
                  newSelectedGroup = sortedGroups[0];
              }

              return {
                  groupsMap: newGroupsMap,
                  groups: sortedGroups,
                  selectedGroup: newSelectedGroup,
                  ...(pendingUpdate || {})
              };
          });
        });
        
        // If we have a selected group, also subscribe to its content
        if (initialSelectedGroup) {
          this.subscribeToGroupContent(initialSelectedGroup.id);
        }

        // Store cleanup for subscriptions
        this.liveSubscriptionCleanup = () => {
          metadataSub.stop();
          // Group content subscriptions cleanup handled separately
        };
      });
    } catch (error) {
      this.setState({ isLoadingHistory: false });
      this.showMessage("Failed to load groups.", "error");
    }
  }

  componentWillUnmount() {
    // Clean up metadata subscription
    this.liveSubscriptionCleanup?.();
    
    // Clean up all group content subscriptions
    this.groupContentSubscriptions.forEach(cleanup => cleanup());
    this.groupContentSubscriptions.clear();
  }

  // Pre-fetch all member data when a group is selected
  async prefetchGroupMemberData(group: Group) {
    if (!group.memberProfiles) {
      group.memberProfiles = new Map();
    }

    // Get all unique pubkeys from members and content authors
    const allPubkeys = new Set<string>();
    group.members.forEach(m => allPubkeys.add(m.pubkey));
    group.content?.forEach(c => allPubkeys.add(c.pubkey));

    // Note: Group write relay initialization removed - NDK outbox model handles this automatically
    const pubkeysArray = Array.from(allPubkeys);

    // Fetch profiles for all members
    const profilePromises = Array.from(allPubkeys).map(async (pubkey) => {
      const profile = await this.props.client.fetchProfile(pubkey);
      return { pubkey, profile };
    });

    const profiles = await Promise.all(profilePromises);
    profiles.forEach(({ pubkey, profile }) => {
      if (!group.memberProfiles!.has(pubkey)) {
        group.memberProfiles!.set(pubkey, {
          pubkey,
          profile,
          has10019: false
        });
      } else {
        group.memberProfiles!.get(pubkey)!.profile = profile;
      }
    });

    // Fetch 10019 events for all members using wallet service
    const walletService = this.props.client.getWalletService();
    const user10019Map = await walletService?.fetchMultipleUsers10019(pubkeysArray) || new Map();
    
    user10019Map.forEach((mintList: any, pubkey: string) => {
      const memberProfile = group.memberProfiles!.get(pubkey);
      if (memberProfile && mintList) {
        memberProfile.has10019 = true;
        memberProfile.lastChecked10019 = Date.now();
        
        // Use wallet service parsing methods instead of manual tag parsing
        const mints = walletService?.parseNutzapMints(mintList) || [];
        const cashuPubkey = walletService?.parseNutzapP2PK(mintList);
        
        if (cashuPubkey) {
          memberProfile.cashuPubkey = cashuPubkey;
          memberProfile.authorizedMints = mints;
        }
      }
    });

    // Mark pubkeys that don't have 10019 events
    pubkeysArray.forEach(pubkey => {
      if (!user10019Map.has(pubkey)) {
        const memberProfile = group.memberProfiles!.get(pubkey);
        if (memberProfile) {
          memberProfile.has10019 = false;
          memberProfile.lastChecked10019 = Date.now();
        }
      }
    });

    // Update the group in state
    this.updateGroupsMap((map) => {
      const updatedGroup = map.get(group.id);
      if (updatedGroup) {
        updatedGroup.memberProfiles = group.memberProfiles;
      }
    });
  }

   updateGroupsMap = (updater: (map: Map<string, Group>) => void) => {
       this.setState((prevState) => {
           // Create a deep enough copy to safely pass to the updater
           const newGroupsMap = new Map(
               Array.from(prevState.groupsMap.entries()).map(([id, group]) => [
                   id,
                   { // Deep copy relevant parts
                       ...group,
                       members: [...group.members],
                       joinRequests: [...group.joinRequests],
                       invites: { ...group.invites },
                       content: group.content ? [...group.content] : []
                   }
               ])
           );

           updater(newGroupsMap); // Let updater modify the copy

           // Re-sort and potentially re-select
           const sortedGroups = Array.from(newGroupsMap.values()).sort(
               (a, b) => b.updated_at - a.updated_at // Use updated_at for sorting
           );
           const currentSelectedId = prevState.selectedGroup?.id;
           let newSelectedGroup = prevState.selectedGroup;
            if (currentSelectedId) {
                newSelectedGroup = newGroupsMap.get(currentSelectedId) || null;
            } else if (sortedGroups.length === 1) {
                 newSelectedGroup = sortedGroups[0];
             }


           return {
               groupsMap: newGroupsMap,
               groups: sortedGroups,
               selectedGroup: newSelectedGroup
           };
       });
   };

  // handleGroupDelete remains the same, but uses updated_at for sorting
  handleGroupDelete = (groupId: string) => {
    this.setState((prevState) => {
      const newGroupsMap = new Map(
        Array.from(prevState.groupsMap.entries()).filter(([id]) => id !== groupId)
      );

      const sortedGroups = Array.from(newGroupsMap.values()).sort(
        (a, b) => b.updated_at - a.updated_at // Consistent sorting
      );

      // If the deleted group was selected, deselect
      const newSelectedGroup = prevState.selectedGroup?.id === groupId ? null : prevState.selectedGroup;

      return {
        groupsMap: newGroupsMap,
        groups: sortedGroups,
        selectedGroup: newSelectedGroup,
      };
    });
  };

  toggleMobileMenu = () => {
    this.setState((state) => ({ isMobileMenuOpen: !state.isMobileMenuOpen }));
  };

   handleGroupSelect = async (group: Group | string) => {
       const groupId = typeof group === 'string' ? group : group.id;
       console.log(`handleGroupSelect: Selecting group ${groupId}`);
       const existingGroup = this.state.groupsMap.get(groupId);

       if (existingGroup) {
           // Unsubscribe from previous group's content
           if (this.state.selectedGroup && this.state.selectedGroup.id !== groupId) {
               const prevSub = this.groupContentSubscriptions.get(this.state.selectedGroup.id);
               if (prevSub) {
                   prevSub();
                   this.groupContentSubscriptions.delete(this.state.selectedGroup.id);
               }
           }
           
           this.setState({
               selectedGroup: existingGroup,
               isMobileMenuOpen: false,
               pendingGroupSelection: null
           });
           
           // Load content if not already loaded
           if (!existingGroup.isFullyLoaded && !existingGroup.isLoading) {
               await this.fetchGroupContent(groupId);
           }
           
           // Subscribe to live updates for this group
           this.subscribeToGroupContent(groupId);
           
           // Prefetch member data after content is loaded
           if (existingGroup.isFullyLoaded) {
               this.prefetchGroupMemberData(existingGroup);
           } else {
               // Wait for content to load, then prefetch member data
               const checkInterval = setInterval(() => {
                   const updatedGroup = this.state.groupsMap.get(groupId);
                   if (updatedGroup && updatedGroup.isFullyLoaded) {
                       clearInterval(checkInterval);
                       this.prefetchGroupMemberData(updatedGroup);
                   }
               }, 500);
               
               // Clear interval after 30 seconds to prevent memory leak
               setTimeout(() => clearInterval(checkInterval), 30000);
           }
       } else {
           // Group metadata hasn't arrived yet, queue it
           this.setState({
               pendingGroupSelection: groupId,
               isMobileMenuOpen: false,
               selectedGroup: null
           });
       }
   };

  showMessage = (
    message: string,
    type: "success" | "error" | "info" = "info"
  ) => {
    this.setState({
      flashMessage: { message, type },
    });
     // Optional: Auto-dismiss after a few seconds
     // setTimeout(() => this.dismissMessage(), 5000);
  };

  dismissMessage = () => {
    this.setState({ flashMessage: null });
  };

  // Checks if a pending selection can now be fulfilled
  private checkPendingSelection = (groupsMap: Map<string, Group>) => {
      const { pendingGroupSelection } = this.state;
      if (pendingGroupSelection) {
          const group = groupsMap.get(pendingGroupSelection);
          if (group) {
              // Prefetch member data for the pending group
              this.prefetchGroupMemberData(group);
              return {
                  selectedGroup: group,
                  pendingGroupSelection: null // Clear the pending flag
              };
          }
      }
      return null; // No pending selection or not found yet
  }

  handleSubdomainSelect = () => {
    // This will be called when user clicks on a subdomain in the list
    // The SubdomainList component will handle the actual navigation
  };

  render() {
    const { client, onLogout } = this.props;
    const { flashMessage, groupsMap, selectedGroup, isMobileMenuOpen, isLoadingHistory, currentSubdomain } = this.state;
    // Always derive groups from the map for consistency
    const groups = Array.from(groupsMap.values()).sort((a, b) => b.updated_at - a.updated_at);

    return (
      <div class="min-h-screen bg-[var(--color-bg-primary)] text-[var(--color-text-primary)]">
        {/* Header */}
        <header class="fixed top-0 left-0 right-0 z-50 h-16 bg-[var(--color-bg-secondary)] border-b border-[var(--color-border)] px-4 lg:px-8">
          <div class="h-full max-w-screen-2xl mx-auto flex items-center justify-between">
            <div class="flex items-center">
              <div class="w-10 mr-4 flex-shrink-0 flex items-center justify-center lg:hidden">
                <BurgerButton
                  isOpen={isMobileMenuOpen}
                  onClick={this.toggleMobileMenu}
                />
              </div>
              <h1 class="text-xl font-bold whitespace-nowrap">Holis👋 Communities Engine</h1>
               {isLoadingHistory && <span class="ml-4 text-sm text-[var(--color-text-secondary)] animate-pulse">Loading history...</span>}
            </div>

            {/* Profile Menu */}
            <ProfileMenu
              client={client}
              onLogout={onLogout}
              showMessage={this.showMessage}
            />
          </div>
        </header>

        {flashMessage && (
          <FlashMessage
            message={flashMessage.message}
            type={flashMessage.type}
            onDismiss={this.dismissMessage}
          />
        )}

        {/* Main container adjusted for fixed header */}
        <div class="flex flex-col xl:flex-row pt-16 min-h-screen">
            {/* Subdomain Sidebar - Far Left */}
            <div class="hidden xl:block xl:w-56 xl:flex-shrink-0 p-3 border-r border-[var(--color-border)]">
                <SubdomainList
                    currentSubdomain={currentSubdomain || ''}
                    onSubdomainSelect={this.handleSubdomainSelect}
                    isLoading={isLoadingHistory}
                />
            </div>

            {/* Groups Sidebar */}
            <div
                class={`
                    fixed lg:sticky top-16 inset-y-0 left-0 z-40 lg:z-auto
                    w-full sm:w-80 lg:w-80 xl:w-80 lg:flex-shrink-0
                    transform transition-transform duration-300 ease-in-out
                    ${isMobileMenuOpen ? 'translate-x-0' : '-translate-x-full lg:translate-x-0'}
                    bg-[var(--color-bg-primary)] lg:bg-transparent border-r border-[var(--color-border)]
                    p-3 lg:p-3
                    overflow-y-auto h-[calc(100vh-4rem)] /* Full height minus header */
                `}
                // style={{ height: 'calc(100vh - 4rem)' }} /* Alt height style */
            >
                {/* Mobile subdomain list */}
                <div class="xl:hidden mb-3">
                    <SubdomainList
                        currentSubdomain={currentSubdomain || ''}
                        onSubdomainSelect={this.handleSubdomainSelect}
                        isLoading={isLoadingHistory}
                    />
                </div>
                
                <CreateGroupForm
                    client={client}
                    updateGroupsMap={this.updateGroupsMap}
                    showMessage={this.showMessage}
                    onGroupCreated={this.handleGroupSelect}
                />
                
                <GroupSidebar
                    groups={groups}
                    selectedGroupId={selectedGroup?.id}
                    onSelectGroup={this.handleGroupSelect} // Pass handleGroupSelect
                    client={client} // Pass client if needed by Sidebar
                    isLoading={isLoadingHistory}
                />
            </div>


            {/* Overlay for mobile */}
             {isMobileMenuOpen && (
                 <div
                     class="fixed inset-0 z-30 bg-black bg-opacity-50 lg:hidden"
                     onClick={this.toggleMobileMenu}
                 />
             )}

            {/* Main Content Area - Remove fixed height and overflow */}
            <main class="flex-grow p-4 lg:p-6"> {/* Removed overflow-y-auto h-[calc(...)] */}
                 {isLoadingHistory ? (
                     <div class="text-center text-[var(--color-text-secondary)] mt-8">
                         <p>Loading historical messages...</p>
                         {/* Optional: add a spinner */}
                     </div>
                 ) : selectedGroup ? (
                    <GroupCard
                        key={selectedGroup.id} // Add key for efficient updates when selection changes
                        group={selectedGroup}
                        client={client}
                        updateGroupsMap={this.updateGroupsMap}
                        showMessage={this.showMessage}
                        onDelete={this.handleGroupDelete}
                    />
                ) : groups.length > 0 ? (
                     <div class="text-center text-[var(--color-text-secondary)] mt-8">
                         <p>Select a channel from the sidebar.</p>
                     </div>
                 ) : (
                    <div class="text-center text-[var(--color-text-secondary)] mt-8">
                        <p>Create a new channel or join one to get started.</p>
                        <p class="text-xs mt-2">(If you recently created/joined a channel, it might still be loading)</p>
                    </div>
                 )}
            </main>
        </div>
      </div>
    );
  }
}