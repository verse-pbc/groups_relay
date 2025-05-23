import { Component, createElement } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { ContentSection } from './ContentSection'
import { MembersSection } from './MembersSection'
import { InviteSection } from './InviteSection'
import { JoinRequestSection } from './JoinRequestSection'
import { InfoSection } from './InfoSection'
import { GroupTabs } from './GroupTabs'

interface GroupContentProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
}

type TabType = 'content' | 'members' | 'invites' | 'requests' | 'info'

interface GroupContentState {
  activeTab: TabType
  deletedInvites: Set<string>
  isAdmin: boolean
}

export class GroupContent extends Component<GroupContentProps, GroupContentState> {
  state: GroupContentState = {
    activeTab: 'content',
    deletedInvites: new Set<string>(),
    isAdmin: false
  }

  async componentDidMount() {
    await this.checkAdminStatus()
  }

  async componentDidUpdate(prevProps: GroupContentProps) {
    if (prevProps.group.id !== this.props.group.id) {
      await this.checkAdminStatus()
    }
  }

  async checkAdminStatus() {
    const user = await this.props.client.ndkInstance.signer?.user()
    if (user?.pubkey) {
      const member = this.props.group.members.find(m => m.pubkey === user.pubkey)
      const isAdmin = member?.roles.includes('Admin') || false
      this.setState({ isAdmin })
    }
  }

  handleTabChange = (tab: TabType) => {
    this.setState({ activeTab: tab })
  }

  handleInviteDelete = (code: string) => {
    this.setState(prevState => ({
      deletedInvites: new Set([...prevState.deletedInvites, code])
    }))
  }

  render() {
    const { group, client, showMessage, updateGroupsMap } = this.props
    const { activeTab, isAdmin } = this.state

    // Filter invites before passing to tabs or sections
    const groupWithFilteredInvites = {
      ...group,
      invites: Object.fromEntries(
        Object.entries(group.invites || {}).filter(([code]) => !this.state.deletedInvites.has(code))
      )
    };

    const sections = [
      // Keep section definitions for mapping, but GroupTabs will use the group prop
      { id: 'content', label: 'Content', component: ContentSection },
      { id: 'members', label: 'Members', component: MembersSection },
      { id: 'invites', label: 'Invites', component: InviteSection },
      { id: 'requests', label: 'Requests', component: JoinRequestSection },
      { id: 'info', label: 'Info', component: InfoSection }
    ] as const; // Use const assertion for type safety

    const ActiveSection = sections.find(s => s.id === activeTab)?.component

    return (
      <div class="flex flex-col p-4 lg:p-6">
        <GroupTabs
          // Pass group object back to GroupTabs
          group={groupWithFilteredInvites}
          activeTab={activeTab}
          onTabChange={this.handleTabChange}
          isAdmin={isAdmin}
        />
        <div class="mt-4">
          {/* Conditionally pass isAdmin only if the component is NOT ContentSection */}
          {ActiveSection && activeTab === 'content' && (
            <ContentSection
              group={group}
              client={client}
              showMessage={showMessage}
            />
          )}
          {ActiveSection && activeTab !== 'content' && (
            createElement(ActiveSection as any, {
              group: group, // Pass original group or filtered one as needed by section
              client: client,
              showMessage: showMessage,
              updateGroupsMap: updateGroupsMap,
              isAdmin: isAdmin, // Pass isAdmin to other sections
              // Pass onInviteDelete specifically to InviteSection if needed
              ...(activeTab === 'invites' && { onInviteDelete: this.handleInviteDelete })
            })
          )}
        </div>
      </div>
    )
  }
}