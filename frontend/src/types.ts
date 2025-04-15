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
}

export interface GroupMember {
  pubkey: string;
  roles: string[];
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
