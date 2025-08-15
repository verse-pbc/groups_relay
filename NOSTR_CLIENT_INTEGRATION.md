# Nostr Client Integration Guide for Groups Relay

This document provides comprehensive instructions for Nostr clients on how to integrate with NIP-29 compliant group relays, including creating groups, managing domains/subdomains, and handling group hierarchies.

## Table of Contents

1. [Overview](#overview)
2. [Connection and Authentication](#connection-and-authentication)
3. [Group Operations](#group-operations)
4. [Domain and Subdomain Structure](#domain-and-subdomain-structure)
5. [Event Kinds Reference](#event-kinds-reference)
6. [Best Practices](#best-practices)
7. [Example Flows](#example-flows)

## Overview

NIP-29 defines relay-based groups where the relay acts as the authority for group management. Groups are identified by `<group-id>` and referenced in events using the `h` tag.

### Key Concepts

- **Group ID**: Unique identifier for a group (e.g., `general`, `bitcoin-dev`)
- **Group Address**: Relay-specific identifier format: `<group-id>'<relay-url>` (e.g., `general'wss://groups.example.com`)
- **Relay Authority**: The relay generates and signs all group state events
- **Role-Based Access**: Admin, Member, and Custom roles with different permissions

## Connection and Authentication

### 1. Initial Connection

```javascript
// Connect to the group relay
const relay = new WebSocket('wss://groups.example.com');

// For private groups, authenticate using NIP-42
relay.send(JSON.stringify([
  "AUTH",
  {
    "id": "<event-id>",
    "pubkey": "<your-pubkey>",
    "created_at": <timestamp>,
    "kind": 22242,
    "tags": [
      ["relay", "wss://groups.example.com"],
      ["challenge", "<challenge-from-relay>"]
    ],
    "content": "",
    "sig": "<signature>"
  }
]));
```

### 2. Subscribing to Groups

```javascript
// Subscribe to a specific group's content
relay.send(JSON.stringify([
  "REQ",
  "<subscription-id>",
  {
    "kinds": [9, 10, 11, 12, 39000, 39001, 39002],
    "#h": ["<group-id>"]
  }
]));

// Subscribe to all groups metadata
relay.send(JSON.stringify([
  "REQ",
  "<subscription-id>",
  {
    "kinds": [39000]
  }
]));
```

## Group Operations

### Creating a Group

Only relay admins or authorized users can create groups:

```javascript
// Send group creation event
const createGroupEvent = {
  "id": "<event-id>",
  "pubkey": "<admin-pubkey>",
  "created_at": <timestamp>,
  "kind": 9007,
  "tags": [
    ["h", "<new-group-id>"],
    ["name", "<group-name>"],
    ["about", "<group-description>"],
    ["picture", "<group-picture-url>"],
    ["private"],  // Optional: make group private
    ["closed"]    // Optional: require approval to join
  ],
  "content": "",
  "sig": "<signature>"
};

relay.send(JSON.stringify(["EVENT", createGroupEvent]));
```

### Joining a Group

For open groups:
```javascript
const joinRequest = {
  "kind": 9021,
  "tags": [
    ["h", "<group-id>"],
    ["relay", "wss://groups.example.com"]
  ],
  "content": "Optional message"
};
```

For closed groups with invite:
```javascript
const joinRequest = {
  "kind": 9021,
  "tags": [
    ["h", "<group-id>"],
    ["relay", "wss://groups.example.com"],
    ["invite", "<invite-code>"]
  ],
  "content": "Optional message"
};
```

### Posting to a Group

```javascript
const groupPost = {
  "kind": 9,  // or 10, 11, 12
  "tags": [
    ["h", "<group-id>"]
  ],
  "content": "Hello group!"
};
```

### Managing Group Members (Admin Only)

```javascript
// Add member
const addMember = {
  "kind": 9000,
  "tags": [
    ["h", "<group-id>"],
    ["p", "<user-pubkey>"]
  ]
};

// Remove member
const removeMember = {
  "kind": 9001,
  "tags": [
    ["h", "<group-id>"],
    ["p", "<user-pubkey>"]
  ]
};

// Set user role
const setRole = {
  "kind": 9006,
  "tags": [
    ["h", "<group-id>"],
    ["p", "<user-pubkey>"],
    ["role", "admin"]  // or "member", "moderator", etc.
  ]
};
```

## Domain and Subdomain Structure

### Hierarchical Group Organization

Groups in NIP-29 can be organized hierarchically using a forward-slash (`/`) naming convention to create domains, subdomains, and sub-groups. This is achieved through the group ID structure:

```
company                          # Top-level domain group
├── company/engineering          # Subdomain under company
│   ├── company/engineering/frontend    # Sub-group under engineering
│   ├── company/engineering/backend     # Sub-group under engineering
│   └── company/engineering/qa          # Sub-group under engineering
├── company/marketing            # Another subdomain
│   ├── company/marketing/social        # Sub-group under marketing
│   └── company/marketing/content       # Sub-group under marketing
└── company/hr                   # Another subdomain
```

### How to Create Domain/Subdomain Structure

#### 1. Creating a Top-Level Domain Group

First, create the main domain group:

```javascript
// Create the top-level domain "company"
const createDomain = {
  "kind": 9007,
  "tags": [
    ["h", "company"],  // This is the domain ID
    ["name", "ACME Company"],
    ["about", "Main company group for all employees"],
    ["picture", "https://example.com/company-logo.png"]
  ],
  "content": "",
  "sig": "<signature>"
};
```

#### 2. Creating Subdomains

Then create subdomain groups with the domain prefix:

```javascript
// Create subdomain "company/engineering"
const createSubdomain = {
  "kind": 9007,
  "tags": [
    ["h", "company/engineering"],  // Note the forward slash
    ["name", "Engineering Team"],
    ["about", "All engineering staff"],
    ["picture", "https://example.com/eng-logo.png"],
    ["closed"]  // Optionally make it closed
  ],
  "content": "",
  "sig": "<signature>"
};
```

#### 3. Creating Sub-groups

Create deeper sub-groups following the same pattern:

```javascript
// Create sub-group "company/engineering/frontend"
const createSubGroup = {
  "kind": 9007,
  "tags": [
    ["h", "company/engineering/frontend"],  // Full path
    ["name", "Frontend Team"],
    ["about", "Frontend developers"],
    ["picture", "https://example.com/frontend-logo.png"]
  ],
  "content": "",
  "sig": "<signature>"
};
```

### Important Rules for Hierarchical Groups

1. **Group IDs use forward slashes** - The `/` character separates hierarchy levels
2. **Each group must be created individually** - Creating `company/engineering` does NOT automatically create `company`
3. **Create from top to bottom** - Create parent groups before child groups
4. **Full path in group ID** - Always use the complete path in the `h` tag

### Step-by-Step Example: Setting Up a Complete Domain Structure

Here's a complete example of setting up a company domain with subdomains:

```javascript
// Step 1: Create the main domain
await relay.send(JSON.stringify(["EVENT", {
  "kind": 9007,
  "tags": [
    ["h", "acme"],
    ["name", "ACME Corporation"],
    ["about", "Main company group"],
    ["picture", "https://acme.com/logo.png"]
  ],
  "content": "",
  "sig": "..."
}]));

// Step 2: Create subdomains
const subdomains = [
  { id: "acme/engineering", name: "Engineering", about: "Engineering teams" },
  { id: "acme/marketing", name: "Marketing", about: "Marketing teams" },
  { id: "acme/sales", name: "Sales", about: "Sales teams" }
];

for (const subdomain of subdomains) {
  await relay.send(JSON.stringify(["EVENT", {
    "kind": 9007,
    "tags": [
      ["h", subdomain.id],
      ["name", subdomain.name],
      ["about", subdomain.about]
    ],
    "content": "",
    "sig": "..."
  }]));
}

// Step 3: Create sub-groups under engineering
const engineeringTeams = [
  { id: "acme/engineering/frontend", name: "Frontend Team" },
  { id: "acme/engineering/backend", name: "Backend Team" },
  { id: "acme/engineering/devops", name: "DevOps Team" }
];

for (const team of engineeringTeams) {
  await relay.send(JSON.stringify(["EVENT", {
    "kind": 9007,
    "tags": [
      ["h", team.id],
      ["name", team.name],
      ["closed"]  // Make sub-groups closed by default
    ],
    "content": "",
    "sig": "..."
  }]));
}
```

### Group ID Naming Convention Summary

| Level | Example | Description |
|-------|---------|-------------|
| Domain | `acme` | Top-level organization |
| Subdomain | `acme/engineering` | Department or major division |
| Sub-group | `acme/engineering/frontend` | Team or project |
| Deep sub-group | `acme/engineering/frontend/react` | Specific technology or sub-team |

#### 2. Automatic Parent Group Membership

When implementing hierarchical groups:

```javascript
// When user joins a subgroup, optionally auto-join parent groups
async function joinGroupWithHierarchy(groupId, relay) {
  const parts = groupId.split('/');
  
  // Join from top-level down
  for (let i = 1; i <= parts.length; i++) {
    const parentGroupId = parts.slice(0, i).join('/');
    await sendJoinRequest(parentGroupId, relay);
  }
}
```

#### 3. Permission Inheritance

Implement permission cascading:

```javascript
function checkGroupPermission(userPubkey, groupId, permission) {
  // Check permission in current group
  if (hasPermission(userPubkey, groupId, permission)) {
    return true;
  }
  
  // Check parent groups
  const parts = groupId.split('/');
  for (let i = parts.length - 1; i > 0; i--) {
    const parentGroupId = parts.slice(0, i).join('/');
    if (hasPermission(userPubkey, parentGroupId, permission)) {
      return true;
    }
  }
  
  return false;
}
```

#### 4. Group Discovery

Implement group browsing by hierarchy:

```javascript
// Fetch all groups and organize by hierarchy
async function getGroupHierarchy(relay) {
  const groups = await fetchAllGroups(relay);
  const hierarchy = {};
  
  groups.forEach(group => {
    const parts = group.id.split('/');
    let current = hierarchy;
    
    parts.forEach((part, index) => {
      if (!current[part]) {
        current[part] = {
          id: parts.slice(0, index + 1).join('/'),
          children: {}
        };
      }
      current = current[part].children;
    });
  });
  
  return hierarchy;
}
```

### Domain-Specific Features

#### 1. Domain Admin Rights

Domain admins should have elevated permissions:

```javascript
function isDomainAdmin(userPubkey, groupId) {
  const domain = groupId.split('/')[0];
  return isGroupAdmin(userPubkey, domain);
}
```

#### 2. Cross-Domain Posting

Allow posting to multiple related groups:

```javascript
const crossPost = {
  "kind": 9,
  "tags": [
    ["h", "company/engineering"],
    ["h", "company/announcements"],
    ["h", "company"]
  ],
  "content": "Important engineering update!"
};
```

#### 3. Domain-Wide Moderation

Implement domain-level moderation:

```javascript
// Domain admin can moderate all subgroups
const deleteEvent = {
  "kind": 9005,
  "tags": [
    ["h", "company/engineering/frontend"],  // Subgroup
    ["e", "<event-id-to-delete>"]
  ]
};
```

## Event Kinds Reference

### User Actions
- `9` - Public chat message
- `10` - Public threaded reply
- `11` - Public forum post
- `12` - Public comment
- `10010` - General public content
- `9021` - Join request
- `9022` - Leave request

### Admin Actions
- `9000` - Add user
- `9001` - Remove user
- `9002` - Edit metadata
- `9005` - Delete event
- `9006` - Set user role
- `9007` - Create group
- `9008` - Delete group
- `9009` - Create invite

### Relay-Generated Events
- `39000` - Group metadata (parameterized replaceable)
- `39001` - Group admins list
- `39002` - Group members list
- `39003` - Supported roles

## Best Practices

### 1. Caching Group Metadata

```javascript
class GroupCache {
  constructor() {
    this.groups = new Map();
    this.members = new Map();
  }
  
  updateFromEvent(event) {
    switch(event.kind) {
      case 39000: // Group metadata
        this.groups.set(event.tags.find(t => t[0] === 'h')[1], {
          name: event.tags.find(t => t[0] === 'name')?.[1],
          about: event.tags.find(t => t[0] === 'about')?.[1],
          picture: event.tags.find(t => t[0] === 'picture')?.[1],
          private: event.tags.some(t => t[0] === 'private'),
          closed: event.tags.some(t => t[0] === 'closed')
        });
        break;
      case 39002: // Members list
        const groupId = event.tags.find(t => t[0] === 'h')[1];
        const members = event.tags.filter(t => t[0] === 'p').map(t => t[1]);
        this.members.set(groupId, members);
        break;
    }
  }
}
```

### 2. Handling Connection Failures

```javascript
class ResilientGroupConnection {
  constructor(relayUrl) {
    this.relayUrl = relayUrl;
    this.reconnectDelay = 1000;
    this.maxReconnectDelay = 30000;
  }
  
  connect() {
    this.ws = new WebSocket(this.relayUrl);
    
    this.ws.onclose = () => {
      setTimeout(() => {
        this.reconnectDelay = Math.min(
          this.reconnectDelay * 2,
          this.maxReconnectDelay
        );
        this.connect();
      }, this.reconnectDelay);
    };
    
    this.ws.onopen = () => {
      this.reconnectDelay = 1000;
      this.resubscribe();
    };
  }
}
```

### 3. Optimistic UI Updates

```javascript
// Show join request immediately, revert if failed
async function joinGroup(groupId) {
  // Optimistic update
  updateUI({ status: 'joining', groupId });
  
  try {
    const response = await sendJoinRequest(groupId);
    if (response.accepted) {
      updateUI({ status: 'member', groupId });
    } else {
      updateUI({ status: 'pending', groupId });
    }
  } catch (error) {
    // Revert optimistic update
    updateUI({ status: 'not-member', groupId });
  }
}
```

### 4. Batch Operations

```javascript
// Batch multiple group operations
async function batchGroupOperations(operations) {
  const events = operations.map(op => ({
    id: generateId(),
    pubkey: userPubkey,
    created_at: Math.floor(Date.now() / 1000),
    kind: op.kind,
    tags: op.tags,
    content: op.content || "",
    sig: signEvent(...)
  }));
  
  // Send all events at once
  for (const event of events) {
    relay.send(JSON.stringify(["EVENT", event]));
  }
}
```

## Example Flows

### Complete Group Creation and Setup Flow

```javascript
async function createAndSetupGroup(groupConfig) {
  // 1. Create the group
  await sendEvent({
    kind: 9007,
    tags: [
      ["h", groupConfig.id],
      ["name", groupConfig.name],
      ["about", groupConfig.description],
      ["picture", groupConfig.picture],
      ...(groupConfig.private ? [["private"]] : []),
      ...(groupConfig.closed ? [["closed"]] : [])
    ]
  });
  
  // 2. Wait for group creation confirmation
  await waitForEvent(39000, { h: groupConfig.id });
  
  // 3. Add initial members
  for (const member of groupConfig.initialMembers) {
    await sendEvent({
      kind: 9000,
      tags: [
        ["h", groupConfig.id],
        ["p", member.pubkey]
      ]
    });
  }
  
  // 4. Set roles for admins
  for (const admin of groupConfig.admins) {
    await sendEvent({
      kind: 9006,
      tags: [
        ["h", groupConfig.id],
        ["p", admin.pubkey],
        ["role", "admin"]
      ]
    });
  }
  
  // 5. Create initial invites if closed
  if (groupConfig.closed && groupConfig.inviteCount) {
    for (let i = 0; i < groupConfig.inviteCount; i++) {
      await sendEvent({
        kind: 9009,
        tags: [
          ["h", groupConfig.id],
          ["uses", "1"]
        ]
      });
    }
  }
}
```

### Hierarchical Group Navigation

```javascript
class GroupNavigator {
  constructor(relay) {
    this.relay = relay;
    this.groupTree = new Map();
  }
  
  async loadGroups() {
    const groups = await this.fetchAllGroups();
    
    groups.forEach(group => {
      const path = group.id.split('/');
      let current = this.groupTree;
      
      path.forEach((segment, index) => {
        if (!current.has(segment)) {
          current.set(segment, {
            id: path.slice(0, index + 1).join('/'),
            metadata: null,
            children: new Map()
          });
        }
        
        if (index === path.length - 1) {
          current.get(segment).metadata = group;
        }
        
        current = current.get(segment).children;
      });
    });
  }
  
  getSubgroups(parentId = '') {
    const path = parentId ? parentId.split('/') : [];
    let current = this.groupTree;
    
    path.forEach(segment => {
      current = current.get(segment)?.children || new Map();
    });
    
    return Array.from(current.values());
  }
  
  getBreadcrumbs(groupId) {
    const path = groupId.split('/');
    const breadcrumbs = [];
    
    path.forEach((segment, index) => {
      breadcrumbs.push({
        id: path.slice(0, index + 1).join('/'),
        name: segment
      });
    });
    
    return breadcrumbs;
  }
}
```

## Security Considerations

1. **Always verify relay signatures** on events kinds 39000-39003
2. **Validate group membership** before showing private content
3. **Check user permissions** before allowing admin actions
4. **Implement rate limiting** for join requests and posts
5. **Cache membership status** but periodically refresh
6. **Handle invite codes securely** - don't expose in URLs
7. **Implement proper error handling** for all group operations

## Testing Checklist

- [ ] Connect to relay and authenticate
- [ ] Fetch and display all public groups
- [ ] Join an open group
- [ ] Post content to a group
- [ ] Leave a group
- [ ] Request to join a closed group
- [ ] Use invite code for closed group
- [ ] Create subgroups (if authorized)
- [ ] Navigate group hierarchy
- [ ] Handle offline/reconnection scenarios
- [ ] Verify permission inheritance
- [ ] Test batch operations
- [ ] Validate optimistic UI updates