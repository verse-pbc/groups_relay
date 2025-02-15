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
}

export class GroupContent extends Component<GroupContentProps, GroupContentState> {
  state = {
    activeTab: 'content' as TabType
  }

  handleTabChange = (tab: TabType) => {
    this.setState({ activeTab: tab })
  }

  render() {
    const { group, client, showMessage, updateGroupsMap } = this.props
    const { activeTab } = this.state

    return (
      <div class="flex flex-col flex-1">
        <GroupTabs
          group={group}
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
              group={group}
              client={client}
              updateGroupsMap={updateGroupsMap}
              showMessage={showMessage}
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