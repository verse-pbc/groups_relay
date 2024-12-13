/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        accent: "#3b82f6",
        success: "#10b981",
        "role-admin": "#1d4ed8",
        "role-moderator": "#6d28d9",
        "role-member": "#10b981",
        "role-invite": "#f97316",
      },
    },
  },
  plugins: [],
};
