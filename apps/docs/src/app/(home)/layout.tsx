import { HomeLayout } from "fumadocs-ui/layouts/home";

import { baseOptions } from "@/lib/layout.shared";

/**
 * Home layout wrapper.
 *
 * @param {object} props - Component props.
 * @param {React.ReactNode} props.children - Page content.
 * @returns {React.JSX.Element} Home layout.
 */
export default function Layout({ children }: { children: React.ReactNode }) {
  return <HomeLayout {...baseOptions}>{children}</HomeLayout>;
}
