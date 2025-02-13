import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { ContentSection } from './ContentSection'
import { MembersSection } from './MembersSection'
import { InviteSection } from './InviteSection'
import { JoinRequestSection } from './JoinRequestSection'
import { GroupTabs } from './GroupTabs'

interface GroupContentProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
}

interface GroupContentState {
  activeTab: 'content' | 'members' | 'invites' | 'requests'
}

export class GroupContent extends Component<GroupContentProps, GroupContentState> {
  state: GroupContentState = {
    activeTab: 'content'
  }

  handleTabChange = (tab: 'content' | 'members' | 'invites' | 'requests') => {
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

        <div class="flex-grow">
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
        </div>
      </div>
    )
  }
}