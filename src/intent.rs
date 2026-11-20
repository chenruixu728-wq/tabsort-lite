use crate::protocol::Tab;
use percent_encoding::percent_decode_str;
use url::Url;

pub struct Purpose {
    pub name: &'static str,
    pub workflow: &'static str,
    title: &'static [&'static str],
    route: &'static [&'static str],
}

impl Purpose {
    pub fn is_action_word(&self, term: &str) -> bool {
        self.name.contains(term) || self.title.contains(&term) || self.route.contains(&term)
    }
}

const PURPOSES: &[Purpose] = &[
    Purpose {
        name: "文献阅读",
        workflow: "科研与分析",
        title: &[
            "论文",
            "文献",
            "期刊",
            "研究综述",
            "paper",
            "preprint",
            "literature review",
            "publication",
        ],
        route: &["abs", "article", "papers", "publications", "doi"],
    },
    Purpose {
        name: "数据与实验",
        workflow: "科研与分析",
        title: &[
            "数据集",
            "实验结果",
            "数据分析",
            "模型评估",
            "dataset",
            "experiment results",
            "data analysis",
            "model evaluation",
        ],
        route: &["datasets", "experiments", "benchmarks"],
    },
    Purpose {
        name: "课程与练习",
        workflow: "学习与进修",
        title: &[
            "课程",
            "讲义",
            "作业",
            "练习",
            "网课",
            "course",
            "lecture",
            "homework",
            "lesson",
            "exercises",
            "tutorial",
        ],
        route: &["courses", "lessons", "lectures", "assignments", "tutorials"],
    },
    Purpose {
        name: "部署与运维",
        workflow: "软件开发",
        title: &[
            "部署",
            "发布流水线",
            "容器镜像",
            "监控面板",
            "deployment",
            "deploy",
            "pipeline",
            "container registry",
            "observability",
        ],
        route: &[
            "deployments",
            "deploy",
            "pipeline",
            "pipelines",
            "actions",
            "monitoring",
        ],
    },
    Purpose {
        name: "问题跟踪",
        workflow: "软件开发",
        title: &[
            "缺陷跟踪",
            "问题跟踪",
            "错误报告",
            "合并请求",
            "issue tracker",
            "issues",
            "bug report",
            "pull requests",
            "merge requests",
        ],
        route: &["issues", "pull", "pulls", "merge requests", "bugs"],
    },
    Purpose {
        name: "代码与实现",
        workflow: "软件开发",
        title: &[
            "源码",
            "代码仓库",
            "代码实现",
            "编程",
            "source code",
            "repository",
            "implementation",
            "code example",
        ],
        route: &[
            "blob",
            "tree",
            "commits",
            "releases",
            "repository",
            "repositories",
        ],
    },
    Purpose {
        name: "技术文档",
        workflow: "软件开发",
        title: &[
            "技术文档",
            "开发文档",
            "接口文档",
            "API 参考",
            "API reference",
            "documentation",
            "developer guide",
            "SDK reference",
        ],
        route: &["docs", "api reference", "developer guide"],
    },
    Purpose {
        name: "岗位与投递",
        workflow: "求职与职业发展",
        title: &[
            "招聘",
            "校招",
            "实习岗位",
            "职位",
            "投递",
            "hiring",
            "jobs",
            "internship",
            "open positions",
            "apply for",
        ],
        route: &["jobs", "careers", "positions", "vacancies", "job detail"],
    },
    Purpose {
        name: "简历与面试",
        workflow: "求职与职业发展",
        title: &[
            "简历",
            "面试",
            "笔试",
            "面经",
            "resume",
            "interview",
            "cover letter",
            "portfolio review",
        ],
        route: &["resumes", "interviews", "cover letter"],
    },
    Purpose {
        name: "设计与素材",
        workflow: "设计与创作",
        title: &[
            "设计稿",
            "原型设计",
            "素材库",
            "配色",
            "字体",
            "组件库",
            "design system",
            "wireframe",
            "mockup",
            "font library",
            "color palette",
        ],
        route: &["design", "mockups", "fonts", "assets"],
    },
    Purpose {
        name: "编辑与协作",
        workflow: "办公与协作",
        title: &[
            "项目计划",
            "协作文档",
            "会议纪要",
            "审批",
            "协作",
            "project plan",
            "meeting notes",
            "shared document",
            "approval",
        ],
        route: &["meetings", "approvals", "collaboration"],
    },
    Purpose {
        name: "购物与比价",
        workflow: "生活事务",
        title: &[
            "购物车",
            "比价",
            "商品详情",
            "订单",
            "shopping cart",
            "price comparison",
            "product listing",
            "my orders",
        ],
        route: &["cart", "checkout", "orders", "products"],
    },
    Purpose {
        name: "申报与缴费",
        workflow: "生活事务",
        title: &[
            "申报",
            "缴费",
            "个人所得税",
            "tax filing",
            "tax return",
            "tax forms",
            "pay a bill",
        ],
        route: &["filing", "payments", "tax forms"],
    },
    Purpose {
        name: "出行与预订",
        workflow: "生活事务",
        title: &[
            "机票",
            "酒店预订",
            "行程",
            "航班",
            "flight search",
            "hotel booking",
            "itinerary",
        ],
        route: &["flights", "hotels", "itinerary", "bookings"],
    },
];

pub struct Page {
    pub subject: String,
    pub description: String,
    pub purpose: Option<&'static Purpose>,
}

pub fn analyze(tab: &Tab) -> Page {
    let url = Url::parse(&tab.url).ok();
    let host = url.as_ref().and_then(Url::host_str).unwrap_or_default();
    let mut subject = clean_title(&tab.title, host)
        .chars()
        .take(165)
        .collect::<String>();
    let parts: Vec<String> = url
        .as_ref()
        .and_then(Url::path_segments)
        .into_iter()
        .flatten()
        .take(10)
        .map(|part| {
            percent_decode_str(part)
                .decode_utf8_lossy()
                .chars()
                .take(65)
                .collect()
        })
        .collect();
    let path = words(&parts.join(" "));
    if let Some(query) = url
        .as_ref()
        .filter(|_| search_engine(host))
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| matches!(key.as_ref(), "q" | "wd"))
                .map(|(_, query)| query.into_owned())
        })
    {
        if !query.trim().is_empty() {
            subject = query.trim().chars().take(165).collect();
        }
    }
    let sparse = subject.is_empty()
        || ["home", "dashboard", "index", "首页", "主页", "欢迎", "登录"]
            .contains(&subject.to_lowercase().as_str());
    if sparse {
        subject.clear();
    }

    let title = words(&subject);
    let purpose = PURPOSES
        .iter()
        .filter_map(|purpose| {
            let title_hits = purpose
                .title
                .iter()
                .filter(|hint| matches(&title, hint))
                .count()
                .min(2);
            let route_hit = purpose.route.iter().any(|hint| matches(&path, hint));
            let score = 4 * title_hits + 3 * usize::from(route_hit);
            (score > 0).then_some((score, purpose))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, purpose)| purpose);

    let mut description = subject.clone();
    // Paths disambiguate generic titles and identify projects without feeding the model a host name.
    let useful_path = parts
        .iter()
        .filter(|part| part.chars().any(char::is_alphabetic))
        .filter(|part| !["topics", "index", "home", "www"].contains(&part.as_str()))
        .take(if sparse { 5 } else { 2 })
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if !useful_path.is_empty() {
        description.push(' ');
        description.extend(useful_path.chars().take(if sparse { 110 } else { 65 }));
    }
    if sparse {
        if let Some(purpose) = purpose {
            description.push(' ');
            description.push_str(purpose.name);
        }
    }
    Page {
        subject,
        description,
        purpose,
    }
}

fn search_engine(host: &str) -> bool {
    ["google.com", "bing.com", "baidu.com", "duckduckgo.com"]
        .iter()
        .any(|domain| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
}

fn clean_title<'a>(title: &'a str, host: &str) -> &'a str {
    let mut title = title.trim();
    while let Some((label, suffix)) = [" - ", " | ", " — ", " · ", " – ", "｜"]
        .iter()
        .filter_map(|separator| title.rsplit_once(separator))
        .max_by_key(|(label, _)| label.len())
    {
        let suffix = suffix.trim().to_ascii_lowercase();
        let brand = [
            "github",
            "gitlab",
            "google search",
            "bing",
            "百度搜索",
            "microsoft edge",
            "youtube",
            "知乎",
            "bilibili",
        ];
        let domain_brand = host
            .split('.')
            .any(|part| part.len() >= 4 && part == suffix);
        if !brand.contains(&suffix.as_str()) && !domain_brand {
            break;
        }
        title = label.trim();
    }
    if host
        .split('.')
        .any(|part| part.len() >= 4 && part.eq_ignore_ascii_case(title))
    {
        return "";
    }
    title
}

fn words(value: &str) -> String {
    format!(
        " {} ",
        value
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    )
}

fn matches(text: &str, hint: &str) -> bool {
    if hint.is_ascii() {
        text.contains(&format!(" {} ", hint.to_ascii_lowercase()))
    } else {
        text.contains(&hint.to_ascii_lowercase())
    }
}
