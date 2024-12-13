import { Component } from 'preact'
import { NostrClient, GroupEventKind } from '../api/nostr_client'
import type { Group, GroupContent, GroupMember } from '../types'
import { GroupList } from './GroupList'
import { CreateGroupForm } from './CreateGroupForm'
import { FlashMessage } from './FlashMessage'
import type { NDKKind } from '@nostr-dev-kit/ndk'

const metadataKinds = [39000, 39001, 39002, 39003];

export interface FlashMessageData {
  message: string
  type: 'success' | 'error' | 'info'
}

export interface AppProps {
  client: NostrClient
}

interface AppState {
  groups: Group[]
  flashMessage: FlashMessageData | null
}

export class App extends Component<AppProps, AppState> {
  private groupsMap: Map<string, Group>

  constructor(props: AppProps) {
    super(props)
    this.state = {
      groups: [],
      flashMessage: null
    }
    this.groupsMap = new Map()
  }

  showMessage = (message: string, type: 'success' | 'error' | 'info' = 'info') => {
    this.setState({ flashMessage: { message, type } })
  }

  dismissMessage = () => {
    this.setState({ flashMessage: null })
  }

  updateGroupsMap = (updater: (map: Map<string, Group>) => void) => {
    updater(this.groupsMap)
    const sortedGroups = Array.from(this.groupsMap.values()).sort((a, b) => b.created_at - a.created_at)
    this.setState({ groups: sortedGroups })
  }

  processEvent = (event: any, groupsMap: Map<string, Group>) => {
    console.log('processing event', event.kind, event)

    // Handle group creation events
    if (event.kind === GroupEventKind.CreateGroup) {
      const groupId = event.tags.find((t: string[]) => t[0] === 'h')?.[1]
      if (!groupId) return

      if (!groupsMap.has(groupId)) {
        const group: Group = {
          id: groupId,
          name: '',
          about: '',
          picture: '',
          private: false,
          closed: false,
          created_at: event.created_at,
          members: [],
          invites: {},
          join_requests: [],
          content: [],
        }
        groupsMap.set(groupId, group)
      }
    }

    // Handle relay-generated metadata events
    if (event.kind >= 39000 && event.kind <= 39003) {
      const groupId = event.tags.find((t: string[]) => t[0] === 'd')?.[1]
      if (!groupId) return

      let group = groupsMap.get(groupId) || {
        id: groupId,
        name: '',
        about: '',
        picture: '',
        private: false,
        closed: false,
        created_at: event.created_at,
        members: [] as GroupMember[],
        invites: {},
        join_requests: [] as string[],
        content: [] as GroupContent[],
      }

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

        case 39003: // Group roles
          // We don't handle this for now
          break
      }

      groupsMap.set(groupId, group)
    }

    // Handle content events
    if (event.kind === 9 || event.kind === 11) {
      const groupId = event.tags.find((t: string[]) => t[0] === 'h')?.[1]
      if (!groupId) return

      const group = groupsMap.get(groupId)
      if (!group) return

      const content: GroupContent = {
        pubkey: event.pubkey,
        kind: event.kind,
        content: event.content,
        created_at: event.created_at,
      }

      group.content = [content, ...(group.content || [])].slice(0, 50)
      groupsMap.set(groupId, { ...group })
    }

    // Handle invite events
    if (event.kind === GroupEventKind.CreateInvite) {
      const groupId = event.tags.find((t: string[]) => t[0] === 'h')?.[1]
      if (!groupId) return

      const group = groupsMap.get(groupId)
      if (!group) return

      const code = event.tags.find((t: string[]) => t[0] === 'code')?.[1]
      const roles = event.tags.find((t: string[]) => t[0] === 'roles')?.[1]?.split(',') || ['member']

      if (code) {
        group.invites = {
          ...group.invites,
          [code]: { roles },
        }
        groupsMap.set(groupId, { ...group })
      }
    }

    // Handle join request events
    if (event.kind === GroupEventKind.JoinRequest) {
      console.log("join request", event)
      const groupId = event.tags.find((t: string[]) => t[0] === 'h')?.[1]
      if (!groupId) return

      const group = groupsMap.get(groupId)
      if (!group) return

      // Add the pubkey to join_requests if not already there
      if (!group.join_requests.includes(event.pubkey)) {
        group.join_requests.push(event.pubkey)
        groupsMap.set(groupId, { ...group })
      }
    }
  }

  async componentDidMount() {
    const fetchMetadataEvents = async () => {
      try {
        const metadataSub = await this.props.client.ndkInstance.fetchEvents({
          kinds: [
            ...metadataKinds,
          ].map(k => k as NDKKind),
        })

        // Process historical events in chronological order
        Array.from(metadataSub)
          .sort((a: any, b: any) => a.created_at - b.created_at)
          .forEach((event: any) => this.processEvent(event, this.groupsMap))

        const sortedGroups = Array.from(this.groupsMap.values()).sort((a, b) => b.created_at - a.created_at)
        this.setState({ groups: sortedGroups })
      } catch (error) {
        console.error('Error fetching metadata events:', error)
      }
    }

    const fetchGroups = async () => {
      try {
        await fetchMetadataEvents()

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
          this.processEvent(event, this.groupsMap)
          const sortedGroups = Array.from(this.groupsMap.values()).sort((a, b) => b.created_at - a.created_at)
          this.setState({ groups: sortedGroups })

          if (event.kind < 30000) {
            console.log('fetching metadata events')
            await new Promise(resolve => setTimeout(resolve, 1000))
            await fetchMetadataEvents()
          }
        })

        return () => {
          sub.stop()
        }
      } catch (error) {
        console.error('Error fetching groups:', error)
      }
    }

    fetchGroups()
  }

  render() {
    const { flashMessage } = this.state

    return (
      <>
        <FlashMessage
          message={flashMessage?.message || null}
          type={flashMessage?.type}
          onDismiss={this.dismissMessage}
        />
        <div class="container mx-auto px-4 py-8">
          <h1 class="text-2xl font-bold text-[var(--color-text-primary)] mb-8">Nostr Groups</h1>

          <div class="flex flex-col lg:flex-row gap-4">
            <div class="lg:w-[240px] flex-shrink-0">
              <CreateGroupForm
                updateGroupsMap={this.updateGroupsMap}
                client={this.props.client}
                showMessage={this.showMessage}
              />
            </div>

            <div class="flex-1">
              <GroupList
                groups={this.state.groups}
                client={this.props.client}
                showMessage={this.showMessage}
              />
            </div>
          </div>
        </div>
      </>
    )
  }
}