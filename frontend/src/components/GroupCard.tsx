import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { InviteSection } from './InviteSection'
import { JoinRequestSection } from './JoinRequestSection'
import { ContentSection } from './ContentSection'

interface GroupCardProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
}

interface GroupCardState {
  isEditingName: boolean
  newName: string
  isEditingAbout: boolean
  newAbout: string
  newMemberPubkey: string
  isAddingMember: boolean
}

export class GroupCard extends Component<GroupCardProps, GroupCardState> {
  constructor(props: GroupCardProps) {
    super(props)
    this.state = {
      isEditingName: false,
      newName: props.group.name || '',
      isEditingAbout: false,
      newAbout: props.group.about || '',
      newMemberPubkey: '',
      isAddingMember: false
    }
  }

  handleNameEdit = () => {
    this.setState({ isEditingName: true })
  }

  handleNameSave = async () => {
    if (!this.state.newName.trim() || this.state.newName === this.props.group.name) {
      this.setState({ isEditingName: false })
      return
    }

    try {
      await this.props.client.updateGroupName(this.props.group.id, this.state.newName)
      this.props.group.name = this.state.newName // Direct modification as discussed
      this.setState({ isEditingName: false })
      this.props.showMessage('Group name updated successfully!', 'success')
    } catch (error) {
      console.error('Failed to update group name:', error)
      this.props.showMessage('Failed to update group name: ' + error, 'error')
    }
  }

  handleAboutEdit = () => {
    this.setState({ isEditingAbout: true })
  }

  handleAboutSave = async () => {
    if (this.state.newAbout === this.props.group.about) {
      this.setState({ isEditingAbout: false })
      return
    }

    try {
      const updatedGroup = { ...this.props.group, about: this.state.newAbout }
      await this.props.client.updateGroupMetadata(updatedGroup)
      this.props.group.about = this.state.newAbout // Direct modification as discussed
      this.setState({ isEditingAbout: false })
      this.props.showMessage('Group description updated successfully!', 'success')
    } catch (error) {
      console.error('Failed to update about:', error)
      this.props.showMessage('Failed to update group description: ' + error, 'error')
    }
  }

  handleMetadataChange = async (field: 'private' | 'closed', value: boolean) => {
    try {
      const updatedGroup = { ...this.props.group, [field]: value }
      await this.props.client.updateGroupMetadata(updatedGroup)
      this.props.group[field] = value
      this.props.showMessage(`Group ${field} setting updated successfully!`, 'success')
    } catch (error) {
      console.error('Error updating metadata:', error)
      this.props.showMessage(`Failed to update group ${field} setting: ` + error, 'error')
    }
  }

  handleRemoveMember = async (pubkey: string) => {
    try {
      await this.props.client.removeMember(this.props.group.id, pubkey)
      this.props.showMessage('Member removed successfully!', 'success')
    } catch (error) {
      console.error('Failed to remove member:', error)
      this.props.showMessage('Failed to remove member: ' + error, 'error')
    }
  }

  handleAddMember = async (e: Event) => {
    e.preventDefault()
    if (!this.state.newMemberPubkey.trim()) return

    this.setState({ isAddingMember: true })
    try {
      await this.props.client.addMember(this.props.group.id, this.state.newMemberPubkey)
      this.setState({ newMemberPubkey: '' })
      this.props.showMessage('Member added successfully!', 'success')
    } catch (error) {
      console.error('Failed to add member:', error)
      this.props.showMessage('Failed to add member: ' + error, 'error')
    } finally {
      this.setState({ isAddingMember: false })
    }
  }

  render() {
    const { group, client } = this.props
    const { isEditingName, newName, isEditingAbout, newAbout, newMemberPubkey, isAddingMember } = this.state

    const truncatePubkey = (pubkey: string) => {
      return pubkey.slice(0, 8) + '...'
    }

    return (
      <article class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] overflow-hidden">
        <div class="flex flex-col lg:flex-row lg:divide-x divide-[var(--color-border)]">
          <div class="lg:w-1/3 flex flex-col">
            <header class="p-3 border-b border-[var(--color-border)] bg-gradient-to-r from-[var(--color-bg-tertiary)] to-[var(--color-bg-secondary)]">
              <div class="flex items-center gap-3">
                {group.picture && (
                  <img
                    src={group.picture}
                    alt={group.name}
                    class="w-8 h-8 rounded object-cover flex-shrink-0"
                    onError={(e) => {
                      (e.target as HTMLImageElement).style.display = 'none'
                    }}
                  />
                )}
                <div class="flex-grow min-w-0">
                  {isEditingName ? (
                    <div class="flex items-center gap-2">
                      <input
                        type="text"
                        value={newName}
                        onInput={e => this.setState({ newName: (e.target as HTMLInputElement).value })}
                        class="flex-1 rounded border border-[var(--color-border)] px-2 py-1 text-xs
                               bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                               focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                               focus:ring-[var(--color-accent)]/10 transition-all"
                        autoFocus
                      />
                      <button
                        onClick={this.handleNameSave}
                        class="px-2 py-1 bg-[var(--color-accent)] text-white rounded text-xs font-medium
                               hover:bg-[var(--color-accent-hover)] active:transform active:translate-y-0.5
                               transition-all disabled:opacity-50"
                      >
                        Save
                      </button>
                    </div>
                  ) : (
                    <h2
                      class="text-base font-semibold text-[var(--color-text-primary)] cursor-pointer px-2 py-1
                             rounded group hover:bg-[var(--color-bg-tertiary)] transition-colors flex items-center gap-1
                             truncate"
                      onClick={this.handleNameEdit}
                    >
                      <span class="truncate">{group.name}</span>
                      <span class="text-xs text-[var(--color-text-secondary)] opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
                        ✏️ edit
                      </span>
                    </h2>
                  )}
                </div>
              </div>
            </header>

            <div class="p-3 flex-grow">
              <div class="space-y-3">
                <div>
                  <span class="text-xs font-medium text-[var(--color-text-secondary)] uppercase tracking-wide">ID:</span>
                  <div class="mt-0.5 text-xs text-[var(--color-text-primary)] break-all">{group.id}</div>
                </div>
                <div>
                  <span class="text-xs font-medium text-[var(--color-text-secondary)] uppercase tracking-wide">About:</span>
                  {isEditingAbout ? (
                    <div class="mt-0.5 flex items-start gap-2">
                      <textarea
                        value={newAbout}
                        onInput={e => this.setState({ newAbout: (e.target as HTMLTextAreaElement).value })}
                        class="flex-1 rounded border border-[var(--color-border)] px-2 py-1 text-xs
                               bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                               focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                               focus:ring-[var(--color-accent)]/10 transition-all resize-none"
                        rows={2}
                        autoFocus
                      />
                      <button
                        onClick={this.handleAboutSave}
                        class="px-2 py-1 bg-[var(--color-accent)] text-white rounded text-xs font-medium
                               hover:bg-[var(--color-accent-hover)] active:transform active:translate-y-0.5
                               transition-all disabled:opacity-50"
                      >
                        Save
                      </button>
                    </div>
                  ) : (
                    <div
                      class="mt-0.5 text-xs text-[var(--color-text-primary)] cursor-pointer group
                             hover:bg-[var(--color-bg-tertiary)] transition-colors rounded px-2 py-1 -mx-2
                             flex items-center gap-1"
                      onClick={this.handleAboutEdit}
                    >
                      {group.about || "No description"}
                      <span class="text-xs text-[var(--color-text-secondary)] opacity-0 group-hover:opacity-100 transition-opacity">
                        ✏️ edit
                      </span>
                    </div>
                  )}
                </div>
                <div>
                  <span class="text-xs font-medium text-[var(--color-text-secondary)] uppercase tracking-wide">Type:</span>
                  <div class="mt-1 flex gap-3">
                    <label class="flex items-center gap-1.5 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={group.private}
                        onChange={() => this.handleMetadataChange('private', !group.private)}
                        class="w-3 h-3 rounded border-[var(--color-border)] text-[var(--color-accent)]
                               focus:ring-[var(--color-accent)] cursor-pointer bg-[var(--color-bg-tertiary)]"
                      />
                      <span class="text-xs text-[var(--color-text-primary)]">Private</span>
                    </label>
                    <label class="flex items-center gap-1.5 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={group.closed}
                        onChange={() => this.handleMetadataChange('closed', !group.closed)}
                        class="w-3 h-3 rounded border-[var(--color-border)] text-[var(--color-accent)]
                               focus:ring-[var(--color-accent)] cursor-pointer bg-[var(--color-bg-tertiary)]"
                      />
                      <span class="text-xs text-[var(--color-text-primary)]">Closed</span>
                    </label>
                  </div>
                </div>
                <div class="space-y-1">
                  <div>
                    <span class="text-xs font-medium text-[var(--color-text-secondary)] uppercase tracking-wide">Created:</span>
                    <div class="text-xs text-[var(--color-text-secondary)]">
                      {new Date(group.created_at * 1000).toLocaleString()}
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div class="lg:w-1/3">
            <div class="p-3">
              <h3 class="flex items-center gap-1 text-sm font-semibold text-[var(--color-text-primary)] mb-2">
                <span class="text-base">👥</span> Members
              </h3>

              <form onSubmit={this.handleAddMember} class="mb-3">
                <div class="flex gap-2">
                  <input
                    type="text"
                    value={newMemberPubkey}
                    onInput={e => this.setState({ newMemberPubkey: (e.target as HTMLInputElement).value })}
                    placeholder="Enter member pubkey"
                    class="flex-1 rounded border border-[var(--color-border)] px-2 py-1 text-xs
                           bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                           focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                           focus:ring-[var(--color-accent)]/10 transition-all font-mono"
                    required
                    disabled={isAddingMember}
                  />
                  <button
                    type="submit"
                    disabled={isAddingMember || !newMemberPubkey.trim()}
                    class="px-2 py-1 bg-[var(--color-accent)] text-white rounded text-xs font-medium
                           hover:bg-[var(--color-accent-hover)] active:transform active:translate-y-0.5
                           transition-all flex items-center gap-1 disabled:opacity-50 whitespace-nowrap"
                  >
                    {isAddingMember ? (
                      <>
                        <span class="animate-spin">⌛</span>
                        Adding...
                      </>
                    ) : (
                      'Add Member'
                    )}
                  </button>
                </div>
              </form>

              <ul class="space-y-2 max-h-[300px] overflow-y-auto">
                {group.members.map(member => (
                  <li key={member.pubkey} class="py-1">
                    <div class="flex items-center justify-between gap-2">
                      <div
                        class="text-xs text-[var(--color-text-secondary)] font-mono hover:text-[var(--color-text-primary)] transition-colors"
                        data-tooltip={member.pubkey}
                      >
                        {truncatePubkey(member.pubkey)}
                      </div>
                      <div class="flex items-center gap-2">
                        <div class="flex flex-wrap gap-1 flex-shrink-0">
                          {member.roles.map(role => {
                            const lower = role.toLowerCase()
                            const [icon] = lower.includes("admin")
                              ? ["⭐"]
                              : lower.includes("moderator")
                                ? ["���"]
                                : ["👤"]
                            return (
                              <span class={`role-badge ${lower.includes("admin") ? "admin" : lower.includes("moderator") ? "moderator" : "member"} text-xs`}>
                                {icon} {role}
                              </span>
                            )
                          })}
                        </div>
                        <button
                          onClick={() => this.handleRemoveMember(member.pubkey)}
                          class="text-red-400 hover:text-red-300 transition-colors flex-shrink-0 p-1"
                          title="Remove member"
                        >
                          ❌
                        </button>
                      </div>
                    </div>
                  </li>
                ))}
              </ul>
            </div>
          </div>

          <div class="lg:w-1/3 flex flex-col">
            <InviteSection group={group} client={client} />
            <JoinRequestSection group={group} client={client} />
            <ContentSection group={group} />
          </div>
        </div>
      </article>
    )
  }
}