import { Component } from 'preact'
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
    if (prevProps.group.id !== this.props.group.id || 
        prevProps.group.members !== this.props.group.members ||
        // Also check when members array changes (for role updates)
        JSON.stringify(prevProps.group.members) !== JSON.stringify(this.props.group.members)) {
      await this.checkAdminStatus()
    }
  }

  async checkAdminStatus() {
    const user = await this.props.client.ndkInstance.signer?.user()
    if (user?.pubkey) {
      // Check if user is a group admin
      const member = this.props.group.members.find(m => m.pubkey === user.pubkey)
      const isGroupAdmin = member?.roles.includes('Admin') || false
      
      // Check if user is the relay admin
      const isRelayAdmin = await this.props.client.checkIsRelayAdmin()
      
      // User is admin if they're either a group admin or relay admin
      const isAdmin = isGroupAdmin || isRelayAdmin
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

    // Remove unused sections array and ActiveSection variable

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
          {/* Keep all components mounted but use CSS to show/hide them */}
          <div style={{ display: activeTab === 'content' ? 'block' : 'none' }}>
            <ContentSection
              group={group}
              client={client}
              showMessage={showMessage}
            />
          </div>
          <div style={{ display: activeTab === 'members' ? 'block' : 'none' }}>
            <MembersSection
              group={group}
              client={client}
              showMessage={showMessage}
              isAdmin={isAdmin}
            />
          </div>
          {isAdmin && (
            <>
              <div style={{ display: activeTab === 'invites' ? 'block' : 'none' }}>
                <InviteSection
                  group={group}
                  client={client}
                  showMessage={showMessage}
                  updateGroupsMap={updateGroupsMap}
                  onInviteDelete={this.handleInviteDelete}
                />
              </div>
              <div style={{ display: activeTab === 'requests' ? 'block' : 'none' }}>
                <JoinRequestSection
                  group={group}
                  client={client}
                  showMessage={showMessage}
                />
              </div>
            </>
          )}
          <div style={{ display: activeTab === 'info' ? 'block' : 'none' }}>
            <InfoSection
              group={group}
              showMessage={showMessage}
            />
          </div>
        </div>
      </div>
    )
  }
}