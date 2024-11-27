import "./style.css";
import NDK, { NDKKind } from "@nostr-dev-kit/ndk";

interface GroupMember {
  pubkey: string;
  roles: string[];
}

interface GroupInvite {
  code: string;
  pubkey?: string;
  roles: string[];
}

interface GroupContent {
  pubkey: string;
  kind: number;
  content: string;
  created_at: number;
}

interface Group {
  id: string;
  name: string;
  about?: string;
  private: boolean;
  closed: boolean;
  members: GroupMember[];
  invites: { [key: string]: GroupInvite };
  join_requests: string[];
  content?: GroupContent[];
}

const ndk = new NDK({
  explicitRelayUrls: ["ws://localhost:8080"],
});

let refreshInterval: ReturnType<typeof setInterval>;

async function init() {
  await ndk.connect();
  console.log("Connected to relay!");

  await fetchGroups();

  refreshInterval = setInterval(fetchGroups, 1000); // Refresh every second
}

async function fetchGroups() {
  try {
    const response = await fetch("/api/groups", {
      headers: {
        Accept: "application/json",
      },
    });

    if (!response.ok) {
      console.error("Server error:", {
        status: response.status,
        statusText: response.statusText,
      });
      const text = await response.text();
      console.error("Response body:", text);
      throw new Error(`HTTP error! status: ${response.status}`);
    }

    const text = await response.text();
    const groups = text ? JSON.parse(text) : [];

    for (const group of groups) {
      group.content = await fetchGroupContent(group.id);
    }

    renderGroups(groups);
  } catch (err) {
    console.error("Failed to fetch groups:", err);
  }
}

async function fetchGroupContent(groupId: string) {
  try {
    const sub = ndk.subscribe(
      {
        kinds: [9, 11, 10010].map((k) => k as NDKKind),
        "#h": [groupId],
      },
      { closeOnEose: true }
    );

    const messages: GroupContent[] = [];

    sub.on("event", (event: any) => {
      messages.push({
        pubkey: event.pubkey,
        kind: event.kind,
        content: event.content,
        created_at: event.created_at,
      });
    });

    await new Promise((resolve) => sub.on("eose", resolve));
    return messages;
  } catch (err) {
    console.error(`Failed to fetch content for group ${groupId}:`, err);
    return [];
  }
}

function renderGroups(groups: Group[]) {
  const app = document.getElementById("app");
  if (!app) return;

  app.innerHTML = `
        <header class="site-header">
            <h1>Groups</h1>
        </header>
        <main>
            ${groups
              .map(
                (group) => `
                <article class="group-card">
                    <header class="group-header">
                        <h2 class="group-name">${group.name}</h2>
                    </header>
                    <section class="group-metadata">
                        <div class="meta-row">
                            <div class="meta-block">
                                <span class="meta-label">ID:</span>
                                <span class="meta-value">${group.id}</span>
                            </div>
                            <div class="meta-block">
                                <span class="meta-label">About:</span>
                                <span class="meta-value">${group.about || "No description"}</span>
                            </div>
                        </div>
                        <div class="meta-row">
                            <div class="meta-block">
                                <span class="meta-label">Type:</span>
                                <span class="meta-value">${group.private ? "Private" : "Public"}, ${group.closed ? "Closed" : "Open"}</span>
                            </div>
                        </div>
                    </section>
                    <section class="card-section members">
                        <h3><span class="icon">👥</span> Members</h3>
                        <ul>
                            ${group.members
                              .map(
                                (member) => `
                                <li>
                                    <div class="list-item-content">
                                        <span class="pubkey">${member.pubkey}</span>
                                        <div class="member-roles">
                                            ${member.roles
                                              .map((role) => {
                                                const lower =
                                                  role.toLowerCase();
                                                const [color, icon] =
                                                  lower.includes("admin")
                                                    ? [
                                                        "var(--role-admin)",
                                                        "⭐",
                                                      ]
                                                    : lower.includes(
                                                          "moderator"
                                                        )
                                                      ? [
                                                          "var(--role-moderator)",
                                                          "🛡",
                                                        ]
                                                      : [
                                                          "var(--role-member)",
                                                          "👤",
                                                        ];
                                                return `<span class="role-badge" style="background: ${color};">${icon} ${role}</span>`;
                                              })
                                              .join(" ")}
                                        </div>
                                    </div>
                                </li>
                            `
                              )
                              .join("\n")}
                        </ul>
                    </section>
                    <section class="card-section invites">
                        <h3><span class="icon">🎟</span> Invites</h3>
                        <ul>
                            ${
                              !group.invites ||
                              Object.entries(group.invites).length === 0
                                ? `<li><div class="list-item-content">No active invites</div></li>`
                                : Object.entries(group.invites)
                                    .map(
                                      ([code, invite]) => `
                                    <li>
                                        <div class="list-item-content">
                                            <div><strong>Code:</strong> ${code}</div>
                                            <div><strong>Accepted by:</strong> ${invite.pubkey || "None"}</div>
                                            <div class="member-roles">
                                                ${invite.roles
                                                  .map(
                                                    (role) =>
                                                      `<span class="role-badge" style="background: var(--role-invite);"> ${role}</span>`
                                                  )
                                                  .join(" ")}
                                            </div>
                                        </div>
                                    </li>
                                `
                                    )
                                    .join("\n")
                            }
                        </ul>
                    </section>
                    <section class="card-section requests">
                        <h3><span class="icon">📨</span> Join Requests</h3>
                        <ul>
                            ${
                              !group.join_requests ||
                              group.join_requests.length === 0
                                ? `<li><div class="list-item-content">No pending requests</div></li>`
                                : group.join_requests
                                    .map(
                                      (pubkey) => `
                                    <li>
                                        <div class="list-item-content">${pubkey}</div>
                                    </li>
                                `
                                    )
                                    .join("\n")
                            }
                        </ul>
                    </section>
                    <section class="card-section content">
                        <h3><span class="icon">💬</span> Content</h3>
                        <ul>
                            ${
                              !group.content || group.content.length === 0
                                ? `<li><div class="list-item-content">No content yet</div></li>`
                                : group.content
                                    .map(
                                      (msg) => `
                                    <li>
                                        <div class="list-item-content">
                                            <div class="content-header">
                                                <span class="pubkey">${msg.pubkey}</span>
                                                <span class="kind-badge">Kind: ${msg.kind}</span>
                                            </div>
                                            <div class="content-body">${msg.content}</div>
                                        </div>
                                    </li>
                                `
                                    )
                                    .join("\n")
                            }
                        </ul>
                    </section>
                </article>
            `
              )
              .join("\n")}
        </main>
    `;
}

init().catch(console.error);

window.addEventListener("unload", () => {
  if (refreshInterval) {
    clearInterval(refreshInterval);
  }
});
