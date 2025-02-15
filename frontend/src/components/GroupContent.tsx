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
}

export class GroupContent extends Component<GroupContentProps, GroupContentState> {
  state: GroupContentState = {
    activeTab: 'content',
    deletedInvites: new Set<string>()
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
    const { activeTab, deletedInvites } = this.state

    // Filter invites for both the tabs and invite section
    const filteredInvites = Object.entries(group.invites || {})
      .filter(([code]) => !deletedInvites.has(code))
      .reduce((acc, [code, invite]) => ({ ...acc, [code]: invite }), {})

    const groupWithFilteredInvites = {
      ...group,
      invites: filteredInvites
    }

    return (
      <div class="flex flex-col flex-1">
        <GroupTabs
          group={groupWithFilteredInvites}
          activeTab={activeTab}
          onTabChange={this.handleTabChange}
        />

        <div class="flex-grow px-6 py-6">
          {activeTab === 'content' && (
            <ContentSection
              group={group}
              client={client}
              showMessage={showMessage}
            />
          )}
          {activeTab === 'members' && (
            <MembersSection
              group={group}
              client={client}
              showMessage={showMessage}
            />
          )}
          {activeTab === 'invites' && (
            <InviteSection
              group={groupWithFilteredInvites}
              client={client}
              updateGroupsMap={updateGroupsMap}
              showMessage={showMessage}
              onInviteDelete={this.handleInviteDelete}
            />
          )}
          {activeTab === 'requests' && (
            <JoinRequestSection
              group={group}
              client={client}
              showMessage={showMessage}
            />
          )}
          {activeTab === 'info' && (
            <InfoSection
              group={group}
              showMessage={showMessage}
            />
          )}
        </div>
      </div>
    )
  }
}