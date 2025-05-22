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

const metadataKinds: NDKKind[] = [39000, 39001, 39002 /*, 39003 removed if not used */];

// Define all kinds to fetch historically and subscribe to live
const relevantKinds: NDKKind[] = [
  ...metadataKinds,
  GroupEventKind.CreateGroup,
  GroupEventKind.CreateInvite,
  GroupEventKind.JoinRequest,
  9, // Chat message
  11, // DM (Note: DMs might require specific handling/decryption not shown here)
  // Add other kinds if needed, e.g., deletions (Kind 5) if you handle them
];

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
      return parts.slice(0, -2).join('.');
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

          case 39001: { // Group admins - Replace logic
              const currentMembers = new Map(mutableGroup.members.map(m => [m.pubkey, { ...m }]));
              const eventAdmins = new Map(
                  event.tags
                      .filter((t: string[]) => t[0] === "p")
                      .map((t: string[]) => [t[1], t.slice(2)]) // [pubkey, roles_array]
              );

              let membersChanged = false;

              // Update existing members or add new ones from the event
              for (const [pubkey, roles] of eventAdmins.entries()) {
                  // Explicitly cast pubkey to string
                  const pubkeyStr = pubkey as string;
                  const member = currentMembers.get(pubkeyStr) || { pubkey: pubkeyStr, roles: [] };
                  // Explicitly type roles as string[]
                  const newRoles = (roles as string[]).length > 0 ? (roles as string[]) : ["Admin"];
                  if (JSON.stringify(member.roles.sort()) !== JSON.stringify(newRoles.sort())) {
                      member.roles = newRoles;
                      currentMembers.set(pubkeyStr, member); // Use casted pubkeyStr
                      membersChanged = true;
                  }
              }

              // Iterate over current members to potentially remove admin role if not in event
              for (const [pubkey, member] of currentMembers.entries()) {
                   if (member.roles.includes('Admin') || member.roles.includes('admin')) { // Check if they were admin
                       if (!eventAdmins.has(pubkey)) { // And are no longer in the admin event
                           const nonAdminRoles = member.roles.filter(r => r.toLowerCase() !== 'admin');
                            if (nonAdminRoles.length === 0) nonAdminRoles.push("Member"); // Fallback to member if no other roles
                            if (JSON.stringify(member.roles.sort()) !== JSON.stringify(nonAdminRoles.sort())) {
                               member.roles = nonAdminRoles;
                               membersChanged = true;
                            }
                       }
                   }
              }


              if (membersChanged) {
                  mutableGroup.members = Array.from(currentMembers.values());
                  updated = true;
              }
              break;
          }


          case 39002: { // Group members metadata - Replace logic (Full replacement)
              const eventMembers = new Map(
                  event.tags
                      .filter((t: string[]) => t[0] === "p")
                      .map((t: string[]) => [t[1], t.slice(2)]) // [pubkey, roles_array]
              );

              const newMemberList: { pubkey: string; roles: string[] }[] = [];
              let listChanged = false;

              for(const [pubkey, roles] of eventMembers.entries()) {
                  // Explicitly type roles as string[]
                  newMemberList.push({ pubkey: pubkey as string, roles: (roles as string[]).length > 0 ? (roles as string[]) : ["Member"] });
              }

              // Simple check if the lists differ (could be more sophisticated)
              if (mutableGroup.members.length !== newMemberList.length ||
                  JSON.stringify(mutableGroup.members.map(m => m.pubkey).sort()) !== JSON.stringify(newMemberList.map(m => m.pubkey).sort())) {
                  listChanged = true;
              }
              // Add role comparison if needed for more accuracy

              if (listChanged) {
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


  // New method for paginated historical fetch
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
                      console.warn(`Oldest timestamp (${batchOldest}) did not change after fetching a full batch. Stopping pagination to prevent potential loop.`);
                       // Option: Try decrementing: oldestTimestampInBatch = batchOldest - 1;
                       // Or just stop:
                      continueFetching = false;
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


  async componentDidMount() {
    this.setState({ isLoadingHistory: true }); // Ensure loading state is true
    const batchSize = 100; // Adjust batch size as needed

    try {
      // Fetch historical data using the new paginated method
      const { groupsMap: initialGroupsMap, latestTimestamp: newestHistoricalTimestamp } =
        await this.fetchHistoricalDataPaginated(
          relevantKinds,
          batchSize
        );

      // Update state with the initially fetched groups
      const sortedGroups = Array.from(initialGroupsMap.values()).sort(
        (a, b) => b.updated_at - a.updated_at // Sort by latest interaction
      );
       const pendingUpdate = this.checkPendingSelection(initialGroupsMap);
       const initialSelectedGroup = pendingUpdate?.selectedGroup || (sortedGroups.length === 1 ? sortedGroups[0] : null);


      this.setState({
        groupsMap: initialGroupsMap,
        groups: sortedGroups,
         selectedGroup: initialSelectedGroup,
         isLoadingHistory: false, // Turn off loading indicator
         ...(pendingUpdate || {})
      }, () => {
        // Step 2: Start live subscription AFTER initial state is set
        console.log("Starting live subscription since:", newestHistoricalTimestamp + 1);
        const liveSub = this.props.client.ndkInstance.subscribe(
          { kinds: relevantKinds, since: newestHistoricalTimestamp + 1 },
          { closeOnEose: false } // Keep it open for live updates
        );

        liveSub.on("event", (event: NDKEvent, _relay: any, _sub: any, _fromCache: any, _optimisticPublish: any) => {
          // Use setState callback to ensure we're working with the latest state
          this.setState((prevState) => {
              // Create a new map instance based on previous state for modification
              let newGroupsMap = new Map(prevState.groupsMap);
              // Process the live event, mutating the new map instance
              newGroupsMap = this.processEvent(event.rawEvent(), newGroupsMap); // Reassign map

              const sortedGroups = Array.from(newGroupsMap.values()).sort(
                  (a, b) => b.updated_at - a.updated_at
              );

               const pendingUpdate = this.checkPendingSelection(newGroupsMap);

              // Update selected group reference if it still exists in the map
              const currentSelectedId = prevState.selectedGroup?.id;
              let newSelectedGroup = prevState.selectedGroup;
              if (currentSelectedId) {
                  newSelectedGroup = newGroupsMap.get(currentSelectedId) || null; // Update or nullify if group disappears
              }
              // If a pending selection was fulfilled, it takes precedence
               if (pendingUpdate?.selectedGroup) {
                   newSelectedGroup = pendingUpdate.selectedGroup;
               } else if (!newSelectedGroup && sortedGroups.length === 1) {
                   // Auto-select if only one group exists and none was selected
                   newSelectedGroup = sortedGroups[0];
               }


              return {
                  groupsMap: newGroupsMap,
                  groups: sortedGroups,
                   selectedGroup: newSelectedGroup,
                   ...(pendingUpdate || {}) // Apply pending selection changes
              };
          });
        });

        // Store cleanup for the live subscription
        this.liveSubscriptionCleanup = () => {
          console.log("Stopping live subscription");
          liveSub.stop();
        };
      });
    } catch (error) {
      console.error("Failed to fetch historical group data:", error);
      this.setState({ isLoadingHistory: false }); // Turn off loading even on error
      this.showMessage("Failed to load historical data.", "error");
      // Handle failure appropriately
    }
  }

  componentWillUnmount() {
    // Only need to clean up the live subscription
    this.liveSubscriptionCleanup?.();
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

   handleGroupSelect = (group: Group | string) => {
       const groupId = typeof group === 'string' ? group : group.id;
       const existingGroup = this.state.groupsMap.get(groupId);

       if (existingGroup) {
           this.setState({
               selectedGroup: existingGroup,
               isMobileMenuOpen: false, // Close menu on selection
               pendingGroupSelection: null // Clear pending
           });
       } else {
           // Group data hasn't arrived yet, queue it
           console.log(`Group ${groupId} not found in map yet, queuing selection.`);
           this.setState({
               pendingGroupSelection: groupId,
               isMobileMenuOpen: false, // Close menu
               selectedGroup: null // Deselect current while waiting
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
               console.log(`Pending selection ${pendingGroupSelection} fulfilled.`);
              return {
                  selectedGroup: group,
                  pendingGroupSelection: null // Clear the pending flag
              };
          }
      }
      return null; // No pending selection or not found yet
  }

  handleSubdomainSelect = (subdomain: string) => {
    // This will be called when user clicks on a subdomain in the list
    // The SubdomainList component will handle the actual navigation
    console.log('Subdomain selected:', subdomain);
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
              <h1 class="text-xl font-bold whitespace-nowrap">Nostr Groups</h1>
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
        <div class="flex flex-col xl:flex-row gap-8 pt-16 min-h-screen">
            {/* Subdomain Sidebar - Far Left */}
            <div class="hidden xl:block xl:w-64 xl:flex-shrink-0 p-4">
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
                    p-4 lg:p-4
                    overflow-y-auto h-[calc(100vh-4rem)] /* Full height minus header */
                `}
                // style={{ height: 'calc(100vh - 4rem)' }} /* Alt height style */
            >
                {/* Mobile subdomain list */}
                <div class="xl:hidden mb-4">
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
                     onGroupCreated={this.handleGroupSelect} // Pass handleGroupSelect
                />
                <hr class="my-4 border-[var(--color-border)]" />
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
            <main class="flex-grow p-4 lg:p-8"> {/* Removed overflow-y-auto h-[calc(...)] */}
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
                         <p>Select a group from the sidebar.</p>
                     </div>
                 ) : (
                    <div class="text-center text-[var(--color-text-secondary)] mt-8">
                        <p>Create a new group or join one to get started.</p>
                        <p class="text-xs mt-2">(If you recently created/joined a group, it might still be loading)</p>
                    </div>
                 )}
            </main>
        </div>
      </div>
    );
  }
}