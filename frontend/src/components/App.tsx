import { Component } from 'preact'
import { NostrClient, GroupEventKind } from '../api/nostr_client'
import type { Group, GroupContent as GroupChatMessage, GroupMember } from '../types'
import { CreateGroupForm } from './CreateGroupForm'
import { GroupCard } from './GroupCard'
import type { NDKKind } from '@nostr-dev-kit/ndk'

const metadataKinds = [39000, 39001, 39002, 39003];

export interface FlashMessageData {
  message: string
  type: 'success' | 'error' | 'info'
}

interface AppProps {
  client: NostrClient
  onLogout: () => void
}

interface AppState {
  groups: Group[]
  flashMessage: FlashMessageData | null
  groupsMap: Map<string, Group>
  selectedGroup: Group | null
  showToast: boolean
  toastMessage: string
  toastType: 'success' | 'error' | 'info'
}

export class App extends Component<AppProps, AppState> {
  private cleanup: (() => void) | null = null

  constructor(props: AppProps) {
    super(props)
    this.state = {
      groups: [],
      flashMessage: null,
      groupsMap: new Map(),
      selectedGroup: null,
      showToast: false,
      toastMessage: '',
      toastType: 'info'
    }
  }

  private getOrCreateGroup = (groupId: string, createdAt: number): Group => {
    if (!this.state.groupsMap.has(groupId)) {
      const group: Group = {
        id: groupId,
        name: '',
        about: '',
        picture: '',
        private: false,
        closed: false,
        created_at: createdAt,
        updated_at: createdAt,
        members: [],
        invites: {},
        joinRequests: [],
        content: [],
      }
      this.state.groupsMap.set(groupId, group)
    }
    if (createdAt > this.state.groupsMap.get(groupId)!.updated_at) {
      this.state.groupsMap.get(groupId)!.updated_at = createdAt
    }
    return this.state.groupsMap.get(groupId)!
  }

  processEvent = (event: any, groupsMap: Map<string, Group>) => {
    console.log('processing event', event.kind, event)

    // Handle group creation events
    if (event.kind === GroupEventKind.CreateGroup) {
      const groupId = event.tags.find((t: string[]) => t[0] === 'h')?.[1]
      if (!groupId) return

      let group = this.getOrCreateGroup(groupId, event.created_at)
      group.created_at = event.created_at
    }

    // Handle relay-generated metadata events
    if (event.kind >= 39000 && event.kind <= 39003) {
      const groupId = event.tags.find((t: string[]) => t[0] === 'd')?.[1]
      if (!groupId) return

      const group = this.getOrCreateGroup(groupId, event.created_at)

      switch (event.kind) {
        case 39000: // Group metadata
          for (const [tag, value] of event.tags) {
            switch (tag) {
              case 'name':
                group.name = value
                break
              case 'about':
                group.about = value
                break
              case 'picture':
                group.picture = value
                break
              case 'private':
                group.private = true
                break
              case 'public':
                group.private = false
                break
              case 'closed':
                group.closed = true
                break
              case 'open':
                group.closed = false
                break
            }
          }
          break

        case 39001: // Group admins
          group.members = group.members.filter(m => !m.roles.includes('admin'))
          event.tags
            .filter((t: string[]) => t[0] === 'p')
            .forEach((t: string[]) => {
              const [_, pubkey, ...roles] = t
              const memberIndex = group.members.findIndex(m => m.pubkey === pubkey)
              if (memberIndex >= 0) {
                group.members[memberIndex].roles = roles
              } else {
                group.members.push({ pubkey, roles } as GroupMember)
              }
            })
          break

        case 39002: // Group members
          const existingPrivilegedMembers = group.members.filter(m =>
            m.roles.some(role => role !== 'member')
          )
          group.members = existingPrivilegedMembers
          event.tags
            .filter((t: string[]) => t[0] === 'p')
            .forEach((t: string[]) => {
              const pubkey = t[1]
              // Only add as member if they don't already have a privileged role
              if (!group.members.some(m => m.pubkey === pubkey)) {
                group.members.push({ pubkey, roles: ['member'] } as GroupMember)
              }
            })
          break
      }

      groupsMap.set(groupId, group)
    }

    // Handle content events
    if (event.kind === 9 || event.kind === 11) {
      const groupId = event.tags.find((t: string[]) => t[0] === 'h')?.[1]
      if (!groupId) return

      const group = this.getOrCreateGroup(groupId, event.created_at)

      const content: GroupChatMessage = {
        id: event.id,
        pubkey: event.pubkey,
        kind: event.kind,
        content: event.content,
        created_at: event.created_at,
      }

      group.content = [content, ...(group.content || [])].slice(0, 50)
      groupsMap.set(groupId, { ...group })
    }
  }

  async componentDidMount() {
    const fetchGroups = async () => {
      try {
        const sub = this.props.client.ndkInstance.subscribe(
          {
            kinds: [
              ...metadataKinds,
              9, 11,
              GroupEventKind.CreateGroup,
              GroupEventKind.CreateInvite,
              GroupEventKind.PutUser,
              GroupEventKind.RemoveUser,
              GroupEventKind.JoinRequest,
            ].map(k => k as NDKKind),
          },
          { closeOnEose: false }
        )

        sub.on('event', async (event: any) => {
          console.log('received event', event.kind)
          this.processEvent(event, this.state.groupsMap)
          const sortedGroups = Array.from(this.state.groupsMap.values()).sort((a, b) => b.created_at - a.created_at)
          this.setState({ groups: sortedGroups })
        })

        // Store the cleanup function
        this.cleanup = () => {
          console.log('Stopping subscription')
          sub.stop()
        }
      } catch (error) {
        console.error('Error fetching groups:', error)
      }
    }

    fetchGroups()
  }

  componentWillUnmount() {
    if (this.cleanup) {
      console.log('Cleaning up subscription')
      this.cleanup()
    }
  }

  updateGroupsMap = (updater: (map: Map<string, Group>) => void) => {
    this.setState(prevState => {
      const newGroupsMap = new Map(prevState.groupsMap)
      updater(newGroupsMap)
      const sortedGroups = Array.from(newGroupsMap.values()).sort((a, b) => b.created_at - a.created_at)
      return {
        groupsMap: newGroupsMap,
        groups: sortedGroups
      }
    })
  }

  handleGroupDelete = (groupId: string) => {
    this.setState(prevState => {
      const newGroupsMap = new Map(prevState.groupsMap)
      newGroupsMap.delete(groupId)
      const sortedGroups = Array.from(newGroupsMap.values()).sort((a, b) => b.created_at - a.created_at)
      return {
        groupsMap: newGroupsMap,
        groups: sortedGroups,
        selectedGroup: null
      }
    })
  }

  handleGroupSelect = (group: Group) => {
    this.setState({ selectedGroup: group })
  }

  showMessage = (message: string, type: 'success' | 'error' | 'info' = 'info') => {
    this.setState({
      showToast: true,
      toastMessage: message,
      toastType: type
    })

    setTimeout(() => {
      this.setState({ showToast: false })
    }, 3000)
  }

  render() {
    const { groups, showToast, toastMessage, toastType } = this.state

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
              <div class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] p-4">
                <CreateGroupForm
                  client={this.props.client}
                  updateGroupsMap={this.updateGroupsMap}
                  showMessage={this.showMessage}
                  onLogout={this.props.onLogout}
                />
              </div>
            </div>

            <div class="flex-1 space-y-4">
              {groups.map(group => (
                <GroupCard
                  key={group.id}
                  group={group}
                  client={this.props.client}
                  showMessage={this.showMessage}
                  onDelete={this.handleGroupDelete}
                />
              ))}
            </div>
          </div>
        </main>

        {showToast && (
          <div class="fixed bottom-4 right-4 p-4 rounded-lg shadow-lg bg-[var(--color-bg-secondary)] border border-[var(--color-border)]">
            <p class={`text-sm ${
              toastType === 'error' ? 'text-red-400' :
              toastType === 'success' ? 'text-green-400' :
              'text-[var(--color-text-primary)]'
            }`}>
              {toastMessage}
            </p>
          </div>
        )}
      </div>
    )
  }
}