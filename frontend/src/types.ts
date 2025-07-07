export interface Group {
  id: string;
  name: string;
  picture?: string;
  about?: string;
  private: boolean;
  closed: boolean;
  broadcast: boolean;
  members: GroupMember[];
  invites: { [key: string]: GroupInvite };
  joinRequests: string[];
  content?: GroupContent[];
  created_at: number;
  updated_at: number;
  // Enhanced state - populated as needed
  memberProfiles?: Map<string, MemberProfile>;
  eventNutzaps?: Map<string, number>; // eventId -> total sats
  // Loading states for lazy loading
  isFullyLoaded?: boolean;
  isLoading?: boolean;
  loadError?: string;
}

export interface GroupMember {
  pubkey: string;
  roles: string[];
}

export interface MemberProfile {
  pubkey: string;
  profile?: any; // NDK profile data
  has10019: boolean;
  cashuPubkey?: string;
  authorizedMints?: string[];
  lastChecked10019?: number; // timestamp
}

export interface GroupInvite {
  code: string;
  pubkey?: string;
  roles: string[];
  id: string;
}

export interface GroupContent {
  id: string;
  pubkey: string;
  kind: number;
  content: string;
  created_at: number;
}
