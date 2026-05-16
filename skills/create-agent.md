# 创建一个Agent

1，确定Agent名：要求用户输入，或根据创建Agent的Channel（比如DiscordChannel ID或Thread ID）
2，在workspace路径下创建该Agent的文件夹
3，在Agent文件夹下创建role.md文件，该文件包含以下内容：
3.1 根据用户输入填写该Agent的角色、任务等
3.2 如果需要定时启动，描述定时启动的具体时间，模式
4，在Agent文件夹下创建memory.md文件，该文件用于保存长期记忆
5，如果用户提出特殊要求，则在Agent文件夹下创建config.toml文件，填写特殊要求。否则缺省使用已经加载到内存的Config（包含使用的LLM等信息）
6，在Agent文件夹下创建sessions文件夹，将按日保存所有与用户的对话
7，Agent创建成功后，返回Agent ID和相关信息给用户

