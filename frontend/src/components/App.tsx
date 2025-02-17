import { Component } from "preact";
import { NostrClient, GroupEventKind } from "../api/nostr_client";
import type {
  Group,
  GroupContent as GroupChatMessage,
} from "../types";
import { CreateGroupForm } from "./CreateGroupForm";
import { GroupCard } from "./GroupCard";
import { FlashMessage } from "./FlashMessage";
import { GroupSidebar } from "./GroupSidebar";
import { BurgerButton } from "./BurgerButton";
import { ProfileMenu } from "./ProfileMenu";

// Define NDKKind type since we can't import it
type NDKKind = number;

const metadataKinds = [39000, 39001, 39002, 39003];

export interface FlashMessageData {
  message: string;
  type: "success" | "error" | "info";
}

interface AppProps {
  client: NostrClient;
  onLogout: () => void;
}

interface AppState {
  groups: Group[];
  flashMessage: FlashMessageData | null;
  groupsMap: Map<string, Group>;
  selectedGroup: Group | null;
  isMobileMenuOpen: boolean;
  pendingGroupSelection: string | null;  // Queue of one for simplicity
}

export class App extends Component<AppProps, AppState> {
  private cleanup: (() => void) | null = null;

  constructor(props: AppProps) {
    super(props);
    this.state = {
      groups: [],
      flashMessage: null,
      groupsMap: new Map(),
      selectedGroup: null,
      isMobileMenuOpen: false,
      pendingGroupSelection: null,
    };
  }

  private getOrCreateGroup = (groupId: string, createdAt: number, groupsMap: Map<string, Group>): Group => {
    const existingGroup = groupsMap.get(groupId);
    if (!existingGroup) {
      const group: Group = {
        id: groupId,
        name: "",
        about: "",
        picture: "",
        private: false,
        closed: false,
        created_at: 0,  // Initialize to 0, will be set when we process the creation event
        updated_at: createdAt,
        members: [],
        invites: {},
        joinRequests: [],
        content: [],
      };
      return group;
    }

    return {
      ...existingGroup,
      updated_at: Math.max(existingGroup.updated_at, createdAt)
    };
  };

  processEvent = (event: any, groupsMap: Map<string, Group>) => {
    const groupId = event.tags.find((t: string[]) => t[0] === "h" || t[0] === "d")?.[1];
    if (!groupId) return groupsMap;

    const group = this.getOrCreateGroup(groupId, event.created_at, groupsMap);

    if (!groupsMap.has(groupId)) {
      groupsMap.set(groupId, group);
    }

    const baseGroup = {
      ...group,
      members: [...group.members],
      joinRequests: [...group.joinRequests],
      invites: { ...group.invites },
      content: group.content ? [...group.content] : []
    };

    let updatedGroup: Group | null = null;

    switch (event.kind) {
      case GroupEventKind.CreateGroup: {
        updatedGroup = {
          ...baseGroup,
          created_at: event.created_at  // Set created_at only for creation events
        };
        break;
      }

      case GroupEventKind.CreateInvite: {
        const codeTag = event.tags.find((t: string[]) => t[0] === "code");
        if (codeTag) {
          const [_, code] = codeTag;
          const invites = { ...baseGroup.invites };
          invites[code] = {
            code,
            pubkey: event.pubkey,
            roles: ["member"],
            id: event.id
          };
          updatedGroup = {
            ...baseGroup,
            invites
          };
        }
        break;
      }

      case 39000: { // Group metadata
        const newMetadata: Partial<Group> = {};
        for (const [tag, value] of event.tags) {
          switch (tag) {
            case "name":
              newMetadata.name = value;
              break;
            case "about":
              newMetadata.about = value;
              break;
            case "picture":
              newMetadata.picture = value;
              break;
            case "private":
              newMetadata.private = true;
              break;
            case "public":
              newMetadata.private = false;
              break;
            case "closed":
              newMetadata.closed = true;
              break;
            case "open":
              newMetadata.closed = false;
              break;
          }
        }

        updatedGroup = {
          ...baseGroup,
          ...newMetadata,
          members: baseGroup.members // Explicitly preserve members
        };
        break;
      }

      case 39001: { // Group admins
        const currentMembers = new Map(baseGroup.members.map(m => [m.pubkey, { ...m }]));

        // Get all pubkeys from the event
        const eventPubkeys = new Set(
          event.tags
            .filter((t: string[]) => t[0] === "p")
            .map((t: string[]) => t[1])
        );

        // Remove members who are no longer in the admin list and have no other roles
        for (const [pubkey, member] of currentMembers.entries()) {
          const isCurrentlyAdmin = member.roles.some(r => r.toLowerCase() === 'admin');
          if (isCurrentlyAdmin) {
            if (!eventPubkeys.has(pubkey)) {
              // This member is no longer an admin
              member.roles = member.roles.filter(r => r.toLowerCase() !== 'admin');
              if (member.roles.length === 0) {
                member.roles = ["Member"];
              }
            }
          }
        }

        // Update roles for members in the event
        event.tags
          .filter((t: string[]) => t[0] === "p")
          .forEach((t: string[]) => {
            const [_, pubkey, ...roles] = t;
            // Create or update member
            const member = currentMembers.get(pubkey) || { pubkey, roles: [] };

            // Ensure we're setting the roles exactly as they come from relay
            member.roles = roles.length > 0 ? roles : ["Member"];
            currentMembers.set(pubkey, member);
          });

        const newMembers = Array.from(currentMembers.values());
        updatedGroup = {
          ...baseGroup,
          members: newMembers
        };
        break;
      }

      case 39002: { // Group members metadata
        // Get all pubkeys from the event
        const eventPubkeys = new Set(
          event.tags
            .filter((t: string[]) => t[0] === "p")
            .map((t: string[]) => t[1])
        );

        // Create a new map with only members that are in the event
        const currentMembers = new Map();

        // First, add existing members that are still in the event
        baseGroup.members.forEach(member => {
          if (eventPubkeys.has(member.pubkey)) {
            currentMembers.set(member.pubkey, { ...member });
          }
        });

        // Then add any new members from the event
        event.tags
          .filter((t: string[]) => t[0] === "p")
          .forEach((t: string[]) => {
            const pubkey = t[1];
            if (!currentMembers.has(pubkey)) {
              currentMembers.set(pubkey, {
                pubkey,
                roles: ["Member"]
              });
            }
          });

        const newMembers = Array.from(currentMembers.values());
        updatedGroup = {
          ...baseGroup,
          members: newMembers,
          joinRequests: baseGroup.joinRequests.filter(pubkey =>
            !newMembers.some(m => m.pubkey === pubkey)
          )
        };
        break;
      }

      case 9:
      case 11: {
        const content: GroupChatMessage = {
          id: event.id,
          pubkey: event.pubkey,
          kind: event.kind,
          content: event.content,
          created_at: event.created_at,
        };

        // Sort content by created_at in ascending order (oldest first)
        const allContent = [...(baseGroup.content || []), content]
          .sort((a, b) => a.created_at - b.created_at)
          .slice(-50);  // Keep last 50 messages

        updatedGroup = {
          ...baseGroup,
          content: allContent
        };
        break;
      }

      case GroupEventKind.JoinRequest: {
        // Only add the join request if the user isn't already a member
        // and if this event is newer than our last metadata update
        if (!baseGroup.members.some(member => member.pubkey === event.pubkey)) {
          const updatedJoinRequests = [...baseGroup.joinRequests];
          if (!updatedJoinRequests.includes(event.pubkey)) {
            updatedJoinRequests.push(event.pubkey);
          }
          updatedGroup = {
            ...baseGroup,
            joinRequests: updatedJoinRequests
          };
        }
        break;
      }

      default: {
        updatedGroup = baseGroup;
        break;
      }
    }

    if (updatedGroup) {
      groupsMap.set(groupId, updatedGroup);
    }

    return groupsMap;
  };

  async componentDidMount() {
    const fetchGroups = async () => {
      try {
        const sub = this.props.client.ndkInstance.subscribe(
          {
            kinds: [
              ...metadataKinds,
              9,
              11,
              GroupEventKind.CreateGroup,
              GroupEventKind.CreateInvite,
              GroupEventKind.JoinRequest,
            ].map((k) => k as NDKKind),
          },
          { closeOnEose: false }
        );

        sub.on("event", async (event: any) => {
          const newGroupsMap = new Map(this.state.groupsMap);
          this.processEvent(event, newGroupsMap);

          const sortedGroups = Array.from(newGroupsMap.values()).sort(
            (a, b) => b.created_at - a.created_at
          );

          // Check if we can fulfill any pending selection
          const pendingUpdate = this.checkPendingSelection(newGroupsMap);

          // Only update the selected group reference if we have one selected
          const newSelectedGroup = pendingUpdate?.selectedGroup || (
            this.state.selectedGroup
              ? newGroupsMap.get(this.state.selectedGroup.id) || this.state.selectedGroup
              : sortedGroups.length === 1 ? sortedGroups[0] : null  // Auto-select first group if it's the only one
          );

          this.setState({
            groupsMap: newGroupsMap,
            groups: sortedGroups,
            selectedGroup: newSelectedGroup,
            ...(pendingUpdate || {})
          });
        });

        this.cleanup = () => {
          sub.stop();
        };
      } catch (error) {
        console.error("Error fetching groups:", error);
      }
    };

    fetchGroups();
  }

  componentWillUnmount() {
    if (this.cleanup) {
      this.cleanup();
    }
  }

  updateGroupsMap = (updater: (map: Map<string, Group>) => void) => {
    this.setState((prevState) => {
      const newGroupsMap = new Map(
        Array.from(prevState.groupsMap.entries()).map(([id, group]) => [
          id,
          {
            ...group,
            members: [...group.members],
            joinRequests: [...group.joinRequests],
            invites: { ...group.invites },
            content: group.content ? [...group.content] : []
          }
        ])
      );

      updater(newGroupsMap);

      // Verify no members were cleared
      newGroupsMap.forEach((group, id) => {
        const prevGroup = prevState.groupsMap.get(id);
        if (prevGroup?.members.length && !group.members.length) {
          group.members = [...prevGroup.members];
        }
      });

      const sortedGroups = Array.from(newGroupsMap.values()).sort(
        (a, b) => b.created_at - a.created_at
      );

      // Auto-select the only group if there's exactly one
      const newSelectedGroup = sortedGroups.length === 1 ? sortedGroups[0] : prevState.selectedGroup;

      return {
        groupsMap: newGroupsMap,
        groups: sortedGroups,
        selectedGroup: newSelectedGroup,
      };
    });
  };

  handleGroupDelete = (groupId: string) => {
    this.setState((prevState) => {
      const newGroupsMap = new Map(
        Array.from(prevState.groupsMap.entries())
          .filter(([id]) => id !== groupId)
          .map(([id, group]) => [
            id,
            {
              ...group,
              members: [...group.members],
              joinRequests: [...group.joinRequests],
              invites: { ...group.invites },
              content: group.content ? [...group.content] : []
            }
          ])
      );

      const sortedGroups = Array.from(newGroupsMap.values()).sort(
        (a, b) => b.created_at - a.created_at
      );

      return {
        groupsMap: newGroupsMap,
        groups: sortedGroups,
        selectedGroup: null,
      };
    });
  };

  toggleMobileMenu = () => {
    this.setState(state => ({ isMobileMenuOpen: !state.isMobileMenuOpen }));
  };

  handleGroupSelect = (group: Group) => {
    // If the group exists in the map, select it immediately
    const existingGroup = this.state.groupsMap.get(group.id);
    if (existingGroup) {
      this.setState({
        selectedGroup: existingGroup,
        isMobileMenuOpen: false,
        pendingGroupSelection: null
      });
    } else {
      // Otherwise, queue it for selection when it becomes available
      this.setState({
        pendingGroupSelection: group.id,
        isMobileMenuOpen: false
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
  };

  dismissMessage = () => {
    this.setState({ flashMessage: null });
  };

  // Add method to check pending selections
  private checkPendingSelection = (groupsMap: Map<string, Group>) => {
    const { pendingGroupSelection } = this.state;
    if (pendingGroupSelection) {
      const group = groupsMap.get(pendingGroupSelection);
      if (group) {
        return {
          selectedGroup: group,
          pendingGroupSelection: null
        };
      }
    }
    return null;
  }

  render() {
    const { client, onLogout } = this.props;
    const { flashMessage, groupsMap, selectedGroup, isMobileMenuOpen } = this.state;
    const groups = Array.from(groupsMap.values()).sort((a, b) => b.updated_at - a.updated_at);

    return (
      <div class="min-h-screen bg-[var(--color-bg-primary)] text-[var(--color-text-primary)]">
        {/* Header */}
        <header class="fixed top-0 left-0 right-0 z-50 h-16 bg-[var(--color-bg-secondary)] border-b border-[var(--color-border)] px-4 lg:px-8">
          <div class="h-full max-w-screen-2xl mx-auto flex items-center justify-between">
            <div class="flex items-center">
              <div class="w-10 mr-4 flex-shrink-0 flex items-center justify-center">
                <BurgerButton
                  isOpen={isMobileMenuOpen}
                  onClick={this.toggleMobileMenu}
                />
              </div>
              <h1 class="text-xl font-bold whitespace-nowrap">Nostr Groups</h1>
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

        <div class="container mx-auto px-8 py-8 lg:py-8 pt-24 lg:pt-24">
          <div class="flex flex-col lg:flex-row gap-8 min-h-[calc(100vh-9rem)]">
            {/* Left Sidebar */}
            <div
              class={`
                fixed lg:relative inset-0 z-40
                w-full lg:w-80 lg:flex-shrink-0
                transform transition-transform duration-300 ease-in-out
                ${isMobileMenuOpen ? 'translate-x-0' : '-translate-x-full lg:translate-x-0'}
                bg-[var(--color-bg-primary)] lg:bg-transparent
                p-4 lg:p-0
                overflow-y-auto
              `}
            >
              <CreateGroupForm
                client={client}
                updateGroupsMap={this.updateGroupsMap}
                showMessage={this.showMessage}
                onGroupCreated={this.handleGroupSelect}
              />
              <GroupSidebar
                groups={groups}
                selectedGroupId={selectedGroup?.id}
                onSelectGroup={this.handleGroupSelect}
                client={client}
              />
            </div>

            {/* Overlay for mobile */}
            {isMobileMenuOpen && (
              <div
                class="fixed inset-0 z-30 bg-black bg-opacity-50 lg:hidden"
                onClick={this.toggleMobileMenu}
              />
            )}

            {/* Main Content */}
            <div class="flex-grow lg:min-h-full">
              {selectedGroup ? (
                <GroupCard
                  group={selectedGroup}
                  client={client}
                  updateGroupsMap={this.updateGroupsMap}
                  showMessage={this.showMessage}
                  onDelete={this.handleGroupDelete}
                />
              ) : (
                <div class="text-center text-[var(--color-text-secondary)] mt-8">
                  <p>Select a group from the sidebar or create a new one to get started</p>
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    );
  }
}
