import { Component } from "preact";
import { NostrClient, GroupEventKind } from "../api/nostr_client";
import type {
  Group,
  GroupContent as GroupChatMessage,
  GroupMember,
} from "../types";
import { CreateGroupForm } from "./CreateGroupForm";
import { GroupCard } from "./GroupCard";
import { FlashMessage } from "./FlashMessage";

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
        created_at: createdAt,
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
        updatedGroup = baseGroup;
        break;
      }

      case GroupEventKind.PutUser: {
        const memberTag = event.tags.find((t: string[]) => t[0] === "p");
        if (memberTag) {
          const [_, pubkey, ...roles] = memberTag;
          const updatedMembers = [...baseGroup.members];
          const memberIndex = updatedMembers.findIndex((m: GroupMember) => m.pubkey === pubkey);

          if (memberIndex >= 0) {
            updatedMembers[memberIndex] = {
              ...updatedMembers[memberIndex],
              roles: [...new Set([...updatedMembers[memberIndex].roles, ...roles])]
            };
          } else {
            updatedMembers.push({ pubkey, roles } as GroupMember);
          }

          updatedGroup = {
            ...baseGroup,
            members: updatedMembers,
            joinRequests: baseGroup.joinRequests.filter(p => p !== pubkey)
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

        event.tags
          .filter((t: string[]) => t[0] === "p")
          .forEach((t: string[]) => {
            const [_, pubkey, ...roles] = t;
            if (currentMembers.has(pubkey)) {
              const member = currentMembers.get(pubkey)!;
              member.roles = [...new Set([...member.roles, ...roles])];
            } else {
              currentMembers.set(pubkey, { pubkey, roles } as GroupMember);
            }
          });

        const newMembers = Array.from(currentMembers.values());
        updatedGroup = {
          ...baseGroup,
          members: newMembers
        };
        break;
      }

      case 39002: { // Group members
        const existingRoles = new Map(
          baseGroup.members.map(member => [member.pubkey, [...member.roles]])
        );

        const newMembers = event.tags
          .filter((t: string[]) => t[0] === "p")
          .map((t: string[]) => {
            const pubkey = t[1];
            return {
              pubkey,
              roles: existingRoles.get(pubkey) || ["member"]
            } as GroupMember;
          });

        if (newMembers.length > 0) {
          updatedGroup = {
            ...baseGroup,
            members: newMembers,
            joinRequests: baseGroup.joinRequests.filter(pubkey =>
              !newMembers.some((m: GroupMember) => m.pubkey === pubkey)
            )
          };
        } else {
          updatedGroup = baseGroup; // Preserve existing state if no new members
        }
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

        updatedGroup = {
          ...baseGroup,
          content: [content, ...(baseGroup.content || [])].slice(0, 50)
        };
        break;
      }

      default: {
        updatedGroup = baseGroup;
        break;
      }
    }

    if (updatedGroup) {
      if (updatedGroup.members.length > 0 || !groupsMap.has(groupId)) {
        groupsMap.set(groupId, updatedGroup);
      } else {
        groupsMap.set(groupId, {
          ...updatedGroup,
          members: group.members // Keep existing members if update would clear them
        });
      }
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
              GroupEventKind.PutUser,
              GroupEventKind.RemoveUser,
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

          this.setState({
            groupsMap: newGroupsMap,
            groups: sortedGroups
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

      return {
        groupsMap: newGroupsMap,
        groups: sortedGroups,
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

  handleGroupSelect = (group: Group) => {
    this.setState({ selectedGroup: group });
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

  render() {
    const { groups, flashMessage } = this.state;

    return (
      <div class="min-h-screen bg-[var(--color-bg-primary)] text-[var(--color-text-primary)]">
        <header class="p-4 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]">
          <div class="max-w-7xl mx-auto">
            <h1 class="text-2xl font-bold">Nostr Groups</h1>
          </div>
        </header>

        <main class="max-w-7xl mx-auto p-4">
          <div class="flex flex-col lg:flex-row gap-4">
            <div class="lg:w-[240px] flex-shrink-0">
              <CreateGroupForm
                client={this.props.client}
                updateGroupsMap={this.updateGroupsMap}
                showMessage={this.showMessage}
                onLogout={this.props.onLogout}
              />
            </div>

            <div class="flex-1 space-y-4">
              {groups.map((group) => (
                <GroupCard
                  key={group.id}
                  group={group}
                  client={this.props.client}
                  showMessage={this.showMessage}
                  onDelete={this.handleGroupDelete}
                  updateGroupsMap={this.updateGroupsMap}
                />
              ))}
            </div>
          </div>
        </main>

        <FlashMessage
          message={flashMessage?.message || null}
          type={flashMessage?.type}
          onDismiss={this.dismissMessage}
        />
      </div>
    );
  }
}
